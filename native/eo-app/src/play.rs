//! `--play`: the player (eo-player) instead of the free-fly camera.
//!
//!   eo-app --play                  window: walk, run, jump, drive (see controls below)
//!   eo-app --bench --play          offscreen: scripted `walk_player` and `drive_player`,
//!                                  prints the player step cost with PASS/FAIL
//!
//! Window controls (the JS game's default bindings, src/input/action-map.js):
//! click the window to capture the mouse (mouse look), Esc releases it, Esc
//! again quits. On foot: W/A/S/D move (camera relative), Shift run (held
//! straight ahead it opens into a sprint), C or Ctrl crouch, Space jump,
//! F enter the car within 2.5 m. Driving: W throttle, S brake then reverse,
//! A/D steer, Space handbrake, C look behind, F get out. V puts a new car
//! 6 m ahead of you (a play-mode helper, not a JS binding). The title bar
//! shows fps, mode, speed and the player step time.
//!
//! The agent renderer draws the player, cars and surrounding traffic. The
//! standard player body comes from the pedestrian pool. Use
//! `--test-character ID` to select one of the demo avatars.
//!
//! Bench flags: `--frames N` (3600 per path), `--segments walk_player,drive_player`,
//! `--no-render` (player only, no GPU), `--collision-threads N` (2), and the
//! ordinary `--width/--height/--msaa/--cache/--no-cache/--no-kit/--no-models/--out`.

use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_player::input::{ActionMap, RawInput};
use eo_player::routes::{self, DrivePilot, WalkPilot};
use eo_player::street_ground::StreetGround;
use eo_player::world::{self as pworld, BuildingSet, CollisionWorld, ElevationGround, GroundProvider};
use eo_player::{CameraView, Mode, Player, StepInput};
use eo_render::camera::Camera;
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, wgpu};
use glam::{DVec3, Vec3};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// The owner's budget for player + physics on the main thread.
const PLAYER_MEDIAN_BUDGET_MS: f64 = 0.5;
const PLAYER_HITCH_BUDGET_MS: f64 = 2.0;

struct PlayOpts {
    interior_eye: bool,
    car_door_animation: bool,
    bench: bool,
    /// Frames per path (default: walk 7200, drive 3600).
    frames: Option<usize>,
    segments: Option<Vec<String>>,
    render: bool,
    collision_threads: usize,
}

/// The hook main.rs calls: None unless `--play` is on the command line.
pub fn intercept(process_start: Instant) -> Option<Result<()>> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if !raw.iter().any(|a| a == "--play") {
        return None;
    }
    Some(parse(&raw).and_then(|(args, opts)| if opts.bench { bench(&args, &opts, process_start) } else { window::run(&args, &opts, process_start) }))
}

/// main.rs's defaults (`parse_args`), field by field: `Args` has no Default.
fn base_args() -> Args {
    Args {
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
        kit_radii: eo_render::kit::KitRadii::default(),
        drive_frames: 3600,
        streets: true,
        // noon, default shadow quality.
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
    }
}

fn parse(raw: &[String]) -> Result<(Args, PlayOpts)> {
    let mut a = base_args();
    let mut test_character = crate::test_characters::Options::default();
    let mut o = PlayOpts { interior_eye: true, car_door_animation: false, bench: false, frames: None, segments: None, render: true, collision_threads: 2 };
    let mut tier = crate::quality::Switches::default(); // `--quality` and the lighting switches
    let mut it = raw.iter();
    while let Some(s) = it.next() {
        if let Some(n) = tier.take(s, it.as_slice().first().map(String::as_str))? {
            it.by_ref().take(n).for_each(drop); // the value it took, if any
            continue;
        }
        let mut val = || it.next().cloned().ok_or_else(|| anyhow!("{s} needs a value"));
        match s.as_str() {
            "--play" => {}
            "--car-door-animation" => o.car_door_animation = true,
            "--no-car-door-animation" => o.car_door_animation = false,
            "--house-eye-camera" => o.interior_eye = true,
            "--no-house-eye-camera" => o.interior_eye = false,
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
            "--bench" => o.bench = true,
            "--capture" => a.capture = true,
            "--frames" => o.frames = Some(val()?.parse()?),
            "--segments" => o.segments = Some(val()?.split(',').map(|s| s.trim().to_string()).collect()),
            "--no-render" => o.render = false,
            "--collision-threads" => o.collision_threads = val()?.parse::<usize>()?.max(1),
            "--blender-camera" => a.look.blender_camera = true,
            "--no-blender-camera" => a.look.blender_camera = false,
            "--water-optics" => a.look.water_optics = true,
            "--cove-detail" => a.cove_detail = true,
            "--cove-canopy" => { a.cove_detail = true; a.look.cove_canopy = true; }
            "--no-cove-canopy" => a.look.cove_canopy = false,
            "--cove-sediment" => { a.cove_detail = true; a.look.cove_sediment = true; }
            "--no-cove-sediment" => a.look.cove_sediment = false,
            "--cove-daylight" => { a.cove_detail = true; a.look.cove_daylight = true; }
            "--no-cove-daylight" => a.look.cove_daylight = false,
            "--cove-pile-contact" => a.cove_detail = true,
            "--no-cove-pile-contact" => {} // resolved with the bay preview settings
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
            "--bridge-detail" | "--no-bridge-detail" => {} // read by `crate::bridge_detail`
            "--width" => a.width = val()?.parse()?,
            "--height" => a.height = val()?.parse()?,
            "--msaa" => a.msaa = val()?.parse()?,
            "--no-vsync" => a.vsync = false,
            "--out" => a.out_dir = Some(val()?.into()),
            "--cache" => a.cache_dir = Some(val()?.into()),
            "--no-cache" => a.no_cache = true,
            "--no-kit" => a.kit = false,
            "--no-models" => a.models = false,
            "--no-streets" => a.streets = false,
            "--no-agents" => a.agents = false, // agents
            "--no-audio" | "--audio-null" => {} // read by audio::AudioRuntime::start
            "--no-photo-mode" | "--no-dof" => {} // read by photo.rs
            "--no-rocketbox" => a.rocketbox = false, // people
            "--no-rocketbox-motions" => eo_agents::motion::set_enabled(false),
            "--no-props" => a.props = false, // props
            "--no-landmark-access" => a.landmark_access = false, // the landmark decks and lifts
            s if a.props2.parse_flag(s) => {}
            "--no-thread-priority" => a.perf.thread_priority = false, // hitch
            "--no-submit-pacing" => a.perf.submit_pacing = false,
            "--no-occlusion" => a.perf.occlusion = false,           // occlusion
            "--no-view-detail" => a.perf.view_detail.enabled = false,
            "--agent-density" => a.agent_density = val()?.parse::<u32>()?.clamp(1, 64),
            // read by main's `groundscale_flags` into the environment switches.
            "--ground-scale" | "--height-scale" | "--people-scale" | "--vehicle-scale" => {
                val()?;
            }
            "--low-camera" | "--default-camera" => {}
            "--no-roof-colours" | "--no-roof-contrast" | "--no-roof-parapet" | "--no-roof-patches" | "--no-roof-plant" => {} // main's `roofs_flags`
            "--no-wall-paint" | "--no-ground-green" | "--no-ground-hills" | "--no-ground-contact" => {} // main's `roofs_flags`
            s if eo_landmarks::suspension::SWITCHES.contains(&s) => {} // eo_landmarks::suspension::features
            "--no-external" => {} // main's `external_flag`
            flag if test_character.parse(flag, &mut val)? => {}
            other => anyhow::bail!("--play: unknown argument {other} (see native/eo-app/src/play.rs)"),
        }
    }
    tier.apply(&mut a);
    a.bench = o.bench;
    crate::apply_cove_look(&mut a, raw);
    a.test_character = test_character.selection(a.agents)?;
    if o.car_door_animation {
        let selected = a.test_character.as_ref().filter(|selected| selected.preview.is_none());
        let has_clips = eo_agents::test_characters::catalogue()?.into_iter().any(|(id, clips)| {
            selected.is_some_and(|selected| selected.id == id)
                && ["car-enter", "car-exit"].iter().all(|name| clips.iter().any(|clip| clip == name))
        });
        anyhow::ensure!(has_clips && o.render, "--car-door-animation needs a demo avatar with both car clips and live rendering");
    }
    Ok((a, o))
}

/// The collision world over the loaded city: every footprint as its JS prism
/// (lifted by the construction lift the renderer uses), standing on the drawn
/// street surface (eo-street: carpet, carriageway, kerbs, sidewalks) when the
/// streets are loaded, else on the elevation grid.
fn collision_world(
    world: &World,
    threads: usize,
    access: bool,
) -> Result<(CollisionWorld, Option<crate::house::Door>)> {
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let mut buildings = BuildingSet::from_city(&world.city, lift);
    if access {
        // the landmarks' decks and lifts (eo-landmarks access.rs), as a server's world has them.
        buildings = buildings.with_decks(eo_net::game::landmark_decks(&world.elevation));
    }
    let ground: Arc<dyn GroundProvider> = match &world.street {
        Some(s) => Arc::new(StreetGround::new(s.world.clone(), world.structures.clone())),
        None => Arc::new(ElevationGround(Arc::new(eo_data::ElevationGrid::load(&world.paths)?))),
    };
    if let Some(house) = &world.house {
        let index = world.city.buildings.iter().position(|b| crate::house::owns(&b.id)).unwrap();
        for ids in buildings.by_tile.values_mut() { ids.retain(|&id| id as usize != index); }
        let ground = Arc::new(crate::house::HouseGround { base: ground, house: house.clone() });
        let mut collision = CollisionWorld::new(ground, Arc::new(buildings), threads);
        let door =
            crate::house::Door::new(house, &world.scene, world.house_vertex_base, &mut collision)?;
        return Ok((collision, Some(door)));
    }
    Ok((CollisionWorld::new(ground, Arc::new(buildings), threads), None))
}

/// Inside the furnished house the lens follows the character's head and posture.
fn player_view(player: &mut Player, cw: &CollisionWorld, world: &World, eye_camera: bool, dt: f64, aspect: f64) -> CameraView {
    let view = player.camera(cw, dt, aspect);
    let feet = player.character.feet();
    if eye_camera && player.mode == Mode::OnFoot && world.house.as_ref().is_some_and(|h| h.contains_feet(feet)) {
        let crouch_drop = 2. * (eo_player::tuning::capsule::CENTER_FROM_FEET_M - player.character.half_extent_y());
        CameraView {
            eye: feet + DVec3::Y * (f64::from(crate::bench::EYE_HEIGHT_M) - crouch_drop),
            forward: DVec3::new(player.yaw.sin() * player.pitch.cos(), -player.pitch.sin(), player.yaw.cos() * player.pitch.cos()),
            body_visible: false,
            ..view
        }
    } else {
        view
    }
}

/// The player's view as the renderer's camera.
fn to_camera(v: &CameraView, aspect: f32, far: f32) -> Camera {
    let f = v.forward.normalize_or(DVec3::Z);
    let yaw = f.x.atan2(-f.z) as f32;
    let pitch = f.y.clamp(-1.0, 1.0).asin() as f32;
    Camera { fov_y_deg: v.fov_deg as f32, far, ..Camera::new(Vec3::new(v.eye.x as f32, v.eye.y as f32, v.eye.z as f32), yaw, pitch, aspect) }
}

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({ "median": percentile(v, 50.0), "p95": percentile(v, 95.0), "max": percentile(v, 100.0), "n": v.len() })
}

/// Arc length to the steepest point of a route, and the route oriented so it is climbed.
fn uphill_route(path: &[[f64; 2]], elevation: &eo_data::ElevationGrid) -> (Vec<[f64; 2]>, f64) {
    let dense = |p: &[[f64; 2]]| {
        let mut out = vec![(p[0], 0.0)];
        let mut s = 0.0;
        for w in p.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            let k = (l / 5.0).ceil().max(1.0) as usize;
            for j in 1..=k {
                let t = j as f64 / k as f64;
                out.push(([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t], s + l * t));
            }
            s += l;
        }
        out
    };
    let d = dense(path);
    let mut best = (0.0, 0usize);
    let mut j = 0;
    for i in 0..d.len() {
        while j < d.len() && d[j].1 - d[i].1 < 20.0 {
            j += 1;
        }
        if j >= d.len() {
            break;
        }
        let g = (elevation.ground(d[j].0[0], d[j].0[1]) - elevation.ground(d[i].0[0], d[i].0[1])) / (d[j].1 - d[i].1);
        if g.abs() > best.0 {
            best = (g.abs(), i);
        }
    }
    let i = best.1;
    let j = (i + 4).min(d.len() - 1);
    let rising = elevation.ground(d[j].0[0], d[j].0[1]) > elevation.ground(d[i].0[0], d[i].0[1]);
    if rising {
        (path.to_vec(), d[i].1)
    } else {
        let mut r = path.to_vec();
        r.reverse();
        let total = routes::polyline_length(path);
        (r, total - d[i].1)
    }
}

/// The part of a polyline from arc length `s0` on.
fn from_arc(path: &[[f64; 2]], s0: f64) -> Vec<[f64; 2]> {
    let mut out = vec![];
    let mut s = 0.0;
    for w in path.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if s + l >= s0 {
            if out.is_empty() {
                let t = ((s0 - s) / l.max(1e-9)).clamp(0.0, 1.0);
                out.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
            }
            out.push(w[1]);
        }
        s += l;
    }
    out
}

struct Gpu<'a> {
    ctx: &'a GpuContext,
    renderer: &'a Renderer,
}

fn settle(g: &Gpu, world: &World, models: &mut ModelRuntime, cam: &Camera, timeout_s: f64) -> Result<f64> {
    let t = Instant::now();
    models.set_camera(cam.eye);
    loop {
        models.pump(g.ctx, g.renderer, world, cam.eye)?;
        g.renderer.update(&g.ctx.queue, cam);
        g.ctx.queue.submit([]);
        let _ = g.ctx.device.poll(wgpu::PollType::Poll);
        if models.settled() || t.elapsed().as_secs_f64() > timeout_s {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

fn bench(args: &Args, opts: &PlayOpts, process_start: Instant) -> Result<()> {
    // the player steps on this thread: above normal, workers below.
    let raised = args.perf.thread_priority && eo_render::hitch::raise_render_thread();
    eo_render::alloc::claim_render_thread();
    eo_render::hitch::set_worker_priority_lowered(args.perf.thread_priority);
    eprintln!("hitch: main thread priority {} (raised: {raised}); thread cycles per ms {:.0}", eo_render::hitch::thread_priority(), eo_render::hitch::cycles_per_ms());
    let mut world = load_world(args)?;
    let t = Instant::now();
    let walk = routes::walk_route(&world.city, &world.elevation);
    let routes_ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!("play bench: data at {:.0} ms, routes {:.0} ms", process_start.elapsed().as_secs_f64() * 1e3, routes_ms);
    let aspect = args.width as f32 / args.height as f32;
    // GPU (optional).
    let gpu_opts = GpuOptions { indirect_validation: args.indirect_validation, force_no_indirect_count: args.no_indirect_count, force_no_count_emulation: args.no_count_emulation, bounded_draws: args.bounded_draws, ..Default::default() };
    let mut gpu_state = None;
    if opts.render {
        let ctx = GpuContext::new(GpuContext::create_instance(&gpu_opts), None, gpu_opts)?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let (renderer, bases) = make_renderer(&ctx, &world, args, format, false)?;
        let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("play-offscreen"),
            size: wgpu::Extent3d { width: args.width, height: args.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = color.create_view(&Default::default());
        let targets = Targets::new(&ctx.device, args.width, args.height, format, args.msaa);
        let start_eye = Vec3::new(walk.path[0][0] as f32, 0.0, walk.path[0][1] as f32);
        let models = ModelRuntime::start(&mut world, args, bases, start_eye, process_start);
        gpu_state = Some((ctx, renderer, color, view, targets, models));
    }
    // the simulated traffic and people around the player, and the player's body and car.
    let mut agents_rt = gpu_state.as_ref().map(|_| crate::agents::AgentRuntime::start(&world, args, [walk.path[0][0], walk.path[0][1], 0.0]));
    // the street props (furniture, trees, signals, signs, overhead, parked cars) in --play too.
    let mut props_rt = gpu_state.as_ref().map(|_| crate::props::PropsRuntime::start(&world, args));
    let mut visual = crate::agents::PlayerVisual::default();
    let mut reports = vec![];
    let mut verdicts = vec![];
    for name in ["walk_player", "drive_player", "car_access"] {
        let access_path = name == "car_access";
        if let Some(only) = &opts.segments {
            if !only.iter().any(|s| s == name) {
                continue;
            }
        } else if access_path {
            continue;
        }
        let driving = name == "drive_player";
        if access_path {
            anyhow::ensure!(opts.render && args.test_character.is_some(), "car_access needs a rendered demo avatar");
        }
        const ACCESS_ENTER: usize = 1;
        const ACCESS_EXIT: usize = 240;
        const ACCESS_FRAMES: usize = 450;
        let (path, steep_s) = uphill_route(&walk.path, &world.elevation);
        // Start far enough before the steepest stretch that the path crosses
        // several blocks on the level and then climbs it.
        let lead = if driving { 500.0 } else { 450.0 };
        let frames = opts.frames.unwrap_or(if access_path { ACCESS_FRAMES } else if driving { 3600 } else { 7200 });
        let path = from_arc(&path, (steep_s - lead).max(0.0));
        let (mut cw, _door) = collision_world(&world, opts.collision_threads, args.landmark_access)?;
        let mut start = DVec3::new(path[0][0], world.elevation.ground(path[0][0], path[0][1]), path[0][1]);
        let t_spawn = Instant::now();
        cw.load_blocking(start, pworld::LOAD_RADIUS_M);
        let spawn_load_ms = t_spawn.elapsed().as_secs_f64() * 1e3;
        start.y = cw.surface_y(start.x, start.z);
        let yaw0 = (path[1][0] - path[0][0]).atan2(path[1][1] - path[0][1]);
        let mut player = Player::new(start, yaw0);
        if access_path {
            let car = player.spawn_vehicle(&mut cw, start.x, start.z, yaw0);
            let q = glam::DQuat::from_rotation_y(yaw0);
            let side = start + q * DVec3::new(1.55, 0.0, 0.14) * eo_data::groundscale::vehicles();
            let feet = player.vehicle_footing(&cw, car, side).ok_or_else(|| anyhow!("car_access: driver's side is blocked"))?;
            player.character.teleport_feet(feet);
            for _ in 0..90 {
                player.fixed_step(&mut cw, &StepInput::default());
            }
        }
        if driving {
            player.spawn_vehicle(&mut cw, path[0][0], path[0][1], yaw0);
            let side = start + DVec3::new(yaw0.cos() * 3.0, 0.0, -yaw0.sin() * 3.0);
            player.character.teleport_feet(DVec3::new(side.x, cw.surface_y(side.x, side.z), side.z));
            for _ in 0..60 {
                player.fixed_step(&mut cw, &StepInput::default());
            }
            player.fixed_step(&mut cw, &StepInput { vehicle_toggle: true, ..Default::default() });
            if !matches!(player.mode, Mode::Driving(_)) {
                anyhow::bail!("drive_player: could not enter the car");
            }
        }
        let mut walk_pilot = WalkPilot::new(path.clone(), true);
        let mut drive_pilot = DrivePilot::new(path.clone(), 50.0 / 3.6);
        let dt = eo_player::tuning::FIXED_STEP_S;
        // Settle the renderer at the start.
        let first = if access_path {
            let at = player.vehicles[0].datum(&cw) + DVec3::Y * 0.8;
            let eye = at + glam::DQuat::from_rotation_y(yaw0) * DVec3::new(4.5, 1.05, 1.8);
            let forward = (at - eye).normalize();
            Camera { far: args.far, fov_y_deg: 45.0, ..Camera::new(eye.as_vec3(), forward.x.atan2(-forward.z) as f32, forward.y.asin() as f32, aspect) }
        } else {
            to_camera(&player_view(&mut player, &cw, &world, args.house_finishes && args.house_furniture && opts.interior_eye, dt, aspect as f64), aspect, args.far)
        };
        let mut settle_ms = 0.0;
        if let Some((ctx, renderer, _c, _view, _targets, models)) = &mut gpu_state {
            let g = Gpu { ctx, renderer };
            settle_ms = settle(&g, &world, models, &first, 600.0)?;
            if let Some(p) = props_rt.as_mut() {
                // until the props around the start are resident.
                let t = Instant::now();
                while !p.settled() && t.elapsed().as_secs_f64() < 600.0 {
                    p.pump(ctx, renderer, &mut models.uploads, first.eye, None, 0.0);
                    models.pump(ctx, renderer, &world, first.eye)?;
                    ctx.queue.submit([]);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                settle_ms += t.elapsed().as_secs_f64() * 1e3;
            }
            if let Some(rt) = agents_rt.as_mut() {
                rt.restart_sim(&world, [first.eye.x as f64, first.eye.z as f64, 0.0], args.agent_density); // around this path's start
                rt.wait_ready(ctx, renderer, [first.eye.x as f64, first.eye.z as f64, 0.0], 300.0);
                if access_path {
                    // Keep the fixed-camera comparison clear of passing traffic.
                    let datum = player.vehicles[0].datum(&cw).as_vec3().to_array();
                    rt.hide_near = vec![(datum, 3.0)];
                }
                if args.test_character.is_some() {
                    anyhow::ensure!(rt.lib.as_ref().is_some_and(|lib| lib.test_player.is_some()),
                        "requested test character failed to load: {:?}", rt.errors);
                }
            }
        }
        let inserted0 = cw.stats.tiles_inserted;
        cw.stats.pump_ms_max = 0.0;
        let mut player_ms = Vec::with_capacity(frames);
        // how long the thread actually ran during the step, and the rest.
        let mut player_ran = Vec::with_capacity(frames);
        // Process page faults during the step (all threads: an indication, not an attribution).
        let mut player_faults: Vec<f64> = Vec::with_capacity(frames);
        let mut pump_ms = Vec::with_capacity(frames);
        let mut frame_cpu_ms = Vec::with_capacity(frames);
        let mut track: Vec<DVec3> = vec![];
        let mut under_ground = 0usize;
        let mut min_gap = f64::MAX;
        let mut four_wheels = 0usize;
        let mut step_ms = Vec::with_capacity(frames);
        let mut camera_ms = Vec::with_capacity(frames);
        let mut subs: Vec<wgpu::SubmissionIndex> = vec![];
        let mut pacer = eo_render::pace::WaitPacer::new(args.perf.submit_pacing);
        let mut car_access = crate::car_access::CarAccess::default();
        let mut max_door = [0.0f32; 2];
        let mut got_in = false;
        let capture_dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        if access_path && args.capture {
            std::fs::create_dir_all(&capture_dir)?;
        }
        for i in 0..frames {
            let mut charged_ms = 0.0; // a prepay the wait did not absorb
            if let Some((ctx, ..)) = &gpu_state {
                if i >= 2 {
                    let (r, w) = pacer.around(&ctx.device, &ctx.queue, || ctx.device.poll(wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }));
                    r.map_err(|e| anyhow!("{e}"))?;
                    charged_ms = w.charged_ms;
                }
            }
            let pf0 = eo_render::hitch::process_page_faults(); // hitch
            let c0 = eo_render::hitch::thread_cycles(); // hitch
            let t0 = Instant::now();
            let input = if access_path {
                StepInput { vehicle_toggle: i == ACCESS_ENTER || i == ACCESS_EXIT, handbrake: true, ..Default::default() }
            } else if driving {
                drive_pilot.input(&player, &cw)
            } else {
                walk_pilot.input(&mut player, &cw)
            };
            if access_path && opts.car_door_animation {
                let lib = agents_rt.as_ref().and_then(|rt| rt.lib.as_deref());
                car_access.advance(&mut player, &mut cw, lib, input, dt);
            } else {
                player.advance(&mut cw, &input, dt);
            }
            if access_path {
                got_in |= matches!(player.mode, Mode::Driving(_));
            }
            let pos = player.position(&cw);
            let tp = Instant::now();
            cw.pump(pos);
            let tc = Instant::now();
            let view = player_view(&mut player, &cw, &world, args.house_finishes && args.house_furniture && opts.interior_eye, dt, aspect as f64);
            let t1 = Instant::now();
            let ran = eo_render::hitch::on_cpu_ms(c0, eo_render::hitch::thread_cycles()); // hitch
            player_faults.push(eo_render::hitch::process_page_faults().saturating_sub(pf0) as f64);
            player_ran.push(ran);
            player_ms.push((t1 - t0).as_secs_f64() * 1e3);
            pump_ms.push((tc - tp).as_secs_f64() * 1e3);
            step_ms.push((tp - t0).as_secs_f64() * 1e3);
            camera_ms.push((t1 - tc).as_secs_f64() * 1e3);
            let gap = pos.y - cw.floor_y(pos.x, pos.z);
            min_gap = min_gap.min(gap);
            // A walker's feet may not sink; a car's datum (body centre minus half
            // its height) rides its springs, so its test is its wheels.
            match player.mode {
                Mode::OnFoot if gap < -0.05 => under_ground += 1,
                Mode::Driving(k) if player.vehicles[k].wheels_in_contact() == 4 => four_wheels += 1,
                _ => {}
            }
            if i % 10 == 0 {
                track.push(pos);
            }
            if let Some((ctx, renderer, color, tv, targets, models)) = &mut gpu_state {
                let cam = if access_path { first.clone() } else { to_camera(&view, aspect, args.far) };
                models.set_camera(cam.eye);
                models.pump(ctx, renderer, &world, cam.eye)?;
                if let (Some(p), Some(rt)) = (props_rt.as_mut(), agents_rt.as_mut()) {
                    // tiles, lamps on the simulation clock, parked cars to the agents.
                    p.pump(ctx, renderer, &mut models.uploads, cam.eye, Some(rt.sim_time), dt);
                    rt.parked = p.parked_agents(rt.lib.as_deref(), cam.eye).to_vec();
                    if let Some(&(center, radius)) = rt.hide_near.first() {
                        rt.parked.retain(|a| (a.pos[0] - center[0]).hypot(a.pos[2] - center[2]) >= radius + 2.5);
                    }
                }
                if let Some(rt) = agents_rt.as_mut() {
                    // agents
                    let transfer = rt.lib.as_ref().and_then(|lib| lib.test_player.as_ref())
                        .and_then(|body| car_access.frame(&player, &cw, body));
                    if let Some(frame) = &transfer {
                        let stage = usize::from(i >= ACCESS_EXIT);
                        max_door[stage] = max_door[stage].max(frame.door);
                    }
                    rt.set_player_with_car_access(&mut visual, &player, &cw, access_path || view.body_visible, dt, transfer.as_ref());
                    rt.pump(ctx, renderer, [pos.x, pos.z, player.yaw], dt);
                }
                renderer.update(&ctx.queue, &cam);
                let mut enc = ctx.device.create_command_encoder(&Default::default());
                let (attach, resolve) = match &targets.msaa {
                    Some(m) => (m, Some(&*tv)),
                    None => (&*tv, None),
                };
                renderer.encode(&mut enc, attach, resolve, &targets.depth, None);
                subs.push(eo_render::pace::submit(&ctx.queue, [enc.finish()])); // counted
                if access_path && args.capture && matches!(i, 0 | 67 | 225 | 299 | 440) {
                    let path = capture_dir.join(format!("car-access-{i:03}.png"));
                    crate::bench::capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &path)?;
                    println!("wrote {}", path.display());
                }
            }
            frame_cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3 + charged_ms);
            if !access_path && ((driving && drive_pilot.done()) || (!driving && walk_pilot.done())) {
                break;
            }
        }
        if let Some((ctx, ..)) = &gpu_state {
            ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
        }
        if access_path && frames >= ACCESS_FRAMES {
            anyhow::ensure!(got_in && player.mode == Mode::OnFoot && !car_access.is_active(),
                "car_access: the two F taps must reach driving and return to foot");
            anyhow::ensure!(!opts.car_door_animation || max_door.iter().all(|&fraction| fraction > 0.9),
                "car_access: both transitions must fully open the driver door: {max_door:?}");
            println!("car access: door maxima {max_door:?}, reached driving and back on foot");
        }
        let distance: f64 = track.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let climbed = track.iter().map(|p| p.y).fold(f64::MIN, f64::max) - track.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        let crossed = walk.intersections.iter().filter(|q| track.iter().any(|p| (p.x - q[0]).hypot(p.z - q[1]) < 10.0)).count();
        // Steepest 20 m stretch actually travelled.
        let mut steepest: f64 = 0.0;
        for a in 0..track.len() {
            for b in a + 1..track.len() {
                let run = (track[b].x - track[a].x).hypot(track[b].z - track[a].z);
                if run >= 20.0 {
                    steepest = steepest.max((track[b].y - track[a].y).abs() / run);
                    break;
                }
            }
        }
        let (med, p95, max) = (percentile(&player_ms, 50.0), percentile(&player_ms, 95.0), percentile(&player_ms, 100.0));
        let pass_med = med <= PLAYER_MEDIAN_BUDGET_MS;
        let pass_max = max <= PLAYER_HITCH_BUDGET_MS;
        let v = |b: bool| if b { "PASS" } else { "FAIL" };
        let inserted = cw.stats.tiles_inserted - inserted0;
        println!(
            "\n[{name}] {} frames, {:.0} m travelled, {:.1} m climbed, steepest 20 m stretch {:.1} %, {} intersections crossed, {} collision tiles streamed in ({} resident)\n  PLAYER STEP (input + fixed step + physics + collision streaming + camera) ms: median {med:.3} [{}]  p95 {p95:.3}  max {max:.3} [{}]   (budget: median <= {PLAYER_MEDIAN_BUDGET_MS} ms, no hitch > {PLAYER_HITCH_BUDGET_MS} ms)\n  split (median / max ms): fixed step {:.3} / {:.3}, collision streaming (owner thread) {:.3} / {:.3} (one tile insert max {:.3}, remove max {:.3}), camera {:.3} / {:.3}; tile cook (worker threads) max {:.2} ms\n  frame CPU incl. render encode/submit ms: median {:.3}  p95 {:.3}  max {:.3}\n  ground: {}; spawn load {spawn_load_ms:.0} ms; render settle {settle_ms:.0} ms",
            player_ms.len(),
            distance,
            climbed,
            steepest * 100.0,
            crossed,
            inserted,
            cw.stats.tiles_resident,
            v(pass_med),
            v(pass_max),
            percentile(&step_ms, 50.0),
            percentile(&step_ms, 100.0),
            percentile(&pump_ms, 50.0),
            percentile(&pump_ms, 100.0),
            cw.stats.insert_ms_max,
            cw.stats.remove_ms_max,
            percentile(&camera_ms, 50.0),
            percentile(&camera_ms, 100.0),
            cw.stats.cook_ms_max,
            percentile(&frame_cpu_ms, 50.0),
            percentile(&frame_cpu_ms, 95.0),
            percentile(&frame_cpu_ms, 100.0),
            if driving {
                format!("all four wheels on the ground {:.1} % of frames, datum min {min_gap:.3} m from the collider under it", four_wheels as f64 * 100.0 / player_ms.len().max(1) as f64)
            } else {
                format!("feet min {min_gap:.3} m from the ground collider, {under_ground} frames more than 5 cm under it")
            },
        );
        // the longest step: did the thread run, or wait?
        let worst = player_ms.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(k, _)| k).unwrap_or(0);
        // A platform without a thread cycle counter has wall times only.
        // Keep the indexed readings for the longest frame and summarize observed pairs.
        let (on_cpu, off_cpu): (Vec<_>, Vec<_>) = player_ran.iter().zip(&player_ms)
            .filter(|(ran, _)| ran.is_finite())
            .map(|(&ran, &wall)| (ran, (wall - ran).max(0.0)))
            .unzip();
        let cpu_detail = match player_ran.get(worst).filter(|ran| ran.is_finite()) {
            Some(ran) => format!("thread ran {ran:.3} ms; off-CPU time per step median {:.3} / max {:.3} ms", percentile(&off_cpu, 50.0), percentile(&off_cpu, 100.0)),
            None => String::from("thread CPU time unavailable"),
        };
        println!(
            "  hitch: longest step frame {worst}: {:.3} ms wall, process page faults during it {:.0} (median a step {:.0}) (fixed step {:.3}, collision {:.3}, camera {:.3}); {cpu_detail}; thread priority {}",
            player_ms.get(worst).copied().unwrap_or(f64::NAN),
            player_faults.get(worst).copied().unwrap_or(f64::NAN),
            percentile(&player_faults, 50.0),
            step_ms.get(worst).copied().unwrap_or(f64::NAN),
            pump_ms.get(worst).copied().unwrap_or(f64::NAN),
            camera_ms.get(worst).copied().unwrap_or(f64::NAN),
            eo_render::hitch::thread_priority(),
        );
        verdicts.push((name, pass_med && pass_max));
        reports.push(json!({
            "name": name,
            "frames": player_ms.len(),
            "rendered": opts.render,
            "distance_m": distance,
            "climbed_m": climbed,
            "steepest_grade_travelled": steepest,
            "intersections_crossed": crossed,
            "collision_tiles_streamed": inserted,
            "player_step_ms": summary(&player_ms),
            "player_step_definition": "input pilot + Player::advance (fixed 1/60 s step: controller or car + Rapier world step) + collision tile pump (owner-thread insert/remove) + camera rig with its probes",
            "collision_pump_ms": summary(&pump_ms),
            "fixed_step_ms": summary(&step_ms),
            "camera_ms": summary(&camera_ms),
            "tile_insert_ms_max": cw.stats.insert_ms_max,
            "tile_remove_ms_max": cw.stats.remove_ms_max,
            "tile_cook_ms_max_worker": cw.stats.cook_ms_max,
            "frame_cpu_ms": summary(&frame_cpu_ms),
            "median_verdict": v(pass_med),
            "max_verdict": v(pass_max),
            "min_ground_gap_m": min_gap,
            "frames_feet_under_ground": under_ground,
            "frames_four_wheels_down": four_wheels,
            // hitch
            "player_step_on_cpu_ms": summary(&on_cpu),
            "player_step_off_cpu_ms": summary(&off_cpu),
            "longest_step": { "frame": worst, "wall_ms": player_ms.get(worst), "ran_ms": player_ran.get(worst), "fixed_step_ms": step_ms.get(worst), "collision_pump_ms": pump_ms.get(worst), "camera_ms": camera_ms.get(worst) },
            "thread_priority": eo_render::hitch::thread_priority(),
        }));
    }
    let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
    std::fs::create_dir_all(&dir)?;
    let out = dir.join("play.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&json!({ "segments": reports, "budget": { "median_ms": PLAYER_MEDIAN_BUDGET_MS, "hitch_ms": PLAYER_HITCH_BUDGET_MS } }))?)?;
    println!("\nwrote {}", out.display());
    let failed: Vec<&str> = verdicts.iter().filter(|v| !v.1).map(|v| v.0).collect();
    if !failed.is_empty() {
        println!("player budget FAIL: {failed:?}");
    }
    Ok(())
}

mod window {
    use super::*;
    use std::collections::HashSet;
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{KeyCode, PhysicalKey};
    use winit::window::{CursorGrabMode, Window, WindowId};

    struct Gfx {
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        ctx: GpuContext,
        renderer: Renderer,
        targets: Targets,
        models: ModelRuntime,
        agents: crate::agents::AgentRuntime, // agents
        visual: crate::agents::PlayerVisual,
        audio: crate::audio::AudioRuntime, // the device (`--no-audio`, `--audio-null`)
        props: crate::props::PropsRuntime,
        pacer: eo_render::pace::WaitPacer, // the driver's submission spike, taken in the swapchain wait
        /// the photo mode panel (made the first time photo mode opens).
        ui: Option<eo_ui::Ui>,
        fade: crate::fade::Fade, // the landmark lift's fade
    }

    struct App {
        interior_eye: bool,
        car_door_animation: bool,
        car_access: crate::car_access::CarAccess,
        world: World,
        args: Args,
        gfx: Option<Gfx>,
        /// photo mode (P), None with `--no-photo-mode`.
        photo: Option<crate::photo::PhotoMode>,
        cw: CollisionWorld,
        door: Option<crate::house::Door>,
        player: Player,
        map: ActionMap,
        raw: RawInput,
        interact_requested: bool,
        vehicle_toggle_requested: bool,
        looking: bool,
        last: Instant,
        fps_frames: u32,
        fps_t: Instant,
        player_ms_max: f64,
        player_ms_sum: f64,
        process_start: Instant,
    }

    pub fn run(args: &Args, opts: &PlayOpts, process_start: Instant) -> Result<()> {
        let world = load_world(args)?;
        let (mut cw, door) = collision_world(&world, opts.collision_threads, args.landmark_access)?;
        let s = crate::bench::spawn(); // the spawn on the scaled plan
        let mut feet = DVec3::new(s[0] as f64, world.elevation.ground(s[0] as f64, s[1] as f64), s[1] as f64);
        if let Some(h) = &world.house { feet = h.entry_position(); }
        cw.load_blocking(feet, pworld::LOAD_RADIUS_M);
        feet.y = cw.surface_y(feet.x, feet.z);
        let yaw = world.house.as_ref().map_or(0., |h| h.entry_yaw());
        let mut player = Player::new(feet, yaw);
        player.spawn_vehicle(&mut cw, feet.x - yaw.cos() * 4.0, feet.z + yaw.sin() * 4.0, yaw);
        let event_loop = EventLoop::new()?;
        let mut app = App {
            interior_eye: args.house_finishes && args.house_furniture && opts.interior_eye,
            car_door_animation: opts.car_door_animation,
            car_access: Default::default(),
            world,
            args: args.clone(),
            gfx: None,
            photo: crate::photo::enabled().then(|| crate::photo::PhotoMode::new(false, args.time_of_day)),
            cw,
            door,
            player,
            map: ActionMap::default(),
            raw: RawInput::default(),
            interact_requested: false,
            vehicle_toggle_requested: false,
            looking: false,
            last: Instant::now(),
            fps_frames: 0,
            fps_t: Instant::now(),
            player_ms_max: 0.0,
            player_ms_sum: 0.0,
            process_start,
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    impl App {
        fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
            let attrs = Window::default_attributes().with_title("Earth Online native - play").with_inner_size(PhysicalSize::new(self.args.width, self.args.height));
            let window = Arc::new(el.create_window(attrs)?);
            let opts = GpuOptions { indirect_validation: self.args.indirect_validation, force_no_indirect_count: self.args.no_indirect_count, force_no_count_emulation: self.args.no_count_emulation, bounded_draws: self.args.bounded_draws, ..Default::default() };
            let instance = GpuContext::create_instance(&opts);
            let surface = instance.create_surface(window.clone())?;
            let ctx = GpuContext::new(instance, Some(&surface), opts)?;
            let size = window.inner_size();
            let caps = surface.get_capabilities(&ctx.adapter);
            let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
            let mut config = surface.get_default_config(&ctx.adapter, size.width.max(1), size.height.max(1)).ok_or_else(|| anyhow!("surface unsupported by adapter"))?;
            config.format = format;
            config.present_mode = if self.args.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            surface.configure(&ctx.device, &config);
            let (renderer, base) = make_renderer(&ctx, &self.world, &self.args, format, false)?;
            let targets = Targets::new(&ctx.device, config.width, config.height, format, self.args.msaa);
            let p = self.player.position(&self.cw);
            let models = ModelRuntime::start(&mut self.world, &self.args, base, Vec3::new(p.x as f32, p.y as f32, p.z as f32), self.process_start);
            let agents = crate::agents::AgentRuntime::start(&self.world, &self.args, [p.x, p.z, self.player.yaw]); // agents
            let props = crate::props::PropsRuntime::start(&self.world, &self.args);
            let pacer = eo_render::pace::WaitPacer::new(self.args.perf.submit_pacing);
            let audio = crate::audio::AudioRuntime::start(&self.world, eo_audio::Output::Device); // audio
            let fade = crate::fade::Fade::new(&ctx.device, format);
            self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, agents, visual: Default::default(), audio, props, pacer, ui: None, fade });
            Ok(())
        }

        fn set_looking(&mut self, on: bool) {
            self.looking = on;
            if let Some(g) = &self.gfx {
                if on {
                    let _ = g.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined));
                } else {
                    let _ = g.window.set_cursor_grab(CursorGrabMode::None);
                }
                g.window.set_cursor_visible(!on);
            }
        }

        fn redraw(&mut self) {
            let now = Instant::now();
            let dt = (now - self.last).as_secs_f64().min(0.1);
            self.last = now;
            let aspect = self.gfx.as_ref().map(|g| g.config.width as f64 / g.config.height.max(1) as f64).unwrap_or(16.0 / 9.0);
            // Player: input, fixed steps, collision streaming, camera.
            let t0 = Instant::now();
            let photo_on = self.photo.as_ref().is_some_and(|p| p.on);
            // photo mode freezes the player's input and the local world (time stands still: the door too).
            let wdt = if photo_on { 0.0 } else { dt };
            if !photo_on {
                let frame = self.map.resolve(&mut self.raw, dt, false);
                let interact = std::mem::take(&mut self.interact_requested)
                    || frame.tapped(eo_player::input::Action::Interact);
                if let Some(door) = &mut self.door {
                    let feet = self.player.position(&self.cw);
                    // (the door waits while the demo avatar gets in or out of the car)
                    if self.player.mode == Mode::OnFoot && !self.car_access.is_active() && interact {
                        door.toggle(feet);
                    }
                    if door.advance(dt, feet, &mut self.world.scene, &mut self.cw)
                        && let Some(g) = &mut self.gfx
                    {
                        g.renderer.look.set_front_door_angle(door.opening_angle());
                        door.upload(&self.world.scene, &g.renderer, &g.ctx.queue);
                    }
                }
                self.player.look(frame.look_yaw, frame.look_pitch);
                // the interact tap (a quick one between redraws too) also rides a landmark lift from its stop
                // (photo mode takes no input: no ride starts in it, and one under way waits).
                let mut input = StepInput { interact, ..StepInput::from_frame(&frame) };
                // F (VehicleToggle): with --car-door-animation a quick tap between redraws too; the
                // demo avatar's animated entry or exit when it can start (car_access.rs), else F as before.
                input.vehicle_toggle |= std::mem::take(&mut self.vehicle_toggle_requested);
                if self.car_door_animation {
                    let library = self.gfx.as_ref().and_then(|g| g.agents.lib.as_deref());
                    self.car_access.advance(&mut self.player, &mut self.cw, library, input, dt);
                } else {
                    self.player.advance(&mut self.cw, &input, dt);
                }
            }
            let pos = self.player.position(&self.cw);
            self.cw.pump(pos);
            let mut view = player_view(&mut self.player, &self.cw, &self.world, self.interior_eye, wdt, aspect);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            self.player_ms_max = self.player_ms_max.max(ms);
            self.player_ms_sum += ms;
            let Some(g) = self.gfx.as_mut() else { return };
            let mut cam = to_camera(&view, aspect as f32, self.args.far);
            if let (Some(ph), true) = (self.photo.as_mut(), photo_on) {
                let mouse = [std::mem::take(&mut self.raw.mouse_dx), std::mem::take(&mut self.raw.mouse_dy)];
                ph.step(&crate::photo::Feed { held: &self.raw.held, mouse, dt, aspect, anchor: crate::photo::anchor_of(pos), cw: Some(&self.cw) });
                if let Some(h) = ph.hour_to_set() {
                    g.renderer.look.set_time_of_day(h); // local only
                }
                g.renderer.photo_mode.set(Some(ph.params()));
                cam = ph.camera(aspect, self.args.far);
                view.body_visible = true; // the player is in the picture
            }
            let (acquired, _) = g.pacer.around(&g.ctx.device, &g.ctx.queue, || g.surface.get_current_texture());
            let surf = match acquired {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                _ => {
                    g.surface.configure(&g.ctx.device, &g.config);
                    return;
                }
            };
            let tv = surf.texture.create_view(&Default::default());
            g.models.set_camera(cam.eye);
            if let Err(e) = g.models.pump(&g.ctx, &g.renderer, &self.world, cam.eye) {
                eprintln!("model streaming: {e:#}");
            }
            g.props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, Some(g.agents.sim_time), wdt);
            g.agents.parked = g.props.parked_agents(g.agents.lib.as_deref(), cam.eye).to_vec();
            let transfer = g.agents.lib.as_ref().and_then(|lib| lib.test_player.as_ref())
                .and_then(|body| self.car_access.frame(&self.player, &self.cw, body));
            g.agents.set_player_with_car_access(&mut g.visual, &self.player, &self.cw, view.body_visible, wdt, transfer.as_ref()); // agents
            g.agents.pump(&g.ctx, &g.renderer, [pos.x, pos.z, self.player.yaw], wdt);
            g.audio.frame(crate::audio::FrameIn::player(&view, dt, &self.player, &self.cw, &mut g.agents, &g.renderer, &self.world.elevation)); // audio
            g.renderer.update(&g.ctx.queue, &cam);
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            let (attach, resolve) = match &g.targets.msaa {
                Some(m) => (m, Some(&tv)),
                None => (&tv, None),
            };
            g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
            if !photo_on {
                g.fade.draw(&mut enc, &tv, self.player.lift_fade()); // a landmark lift ride (photo mode's free camera is not faded)
            }
            if let (Some(ph), true) = (self.photo.as_ref(), photo_on) {
                // the photo panel over the picture (never in a saved photo).
                let ui = g.ui.get_or_insert_with(|| {
                    let mut u = eo_ui::Ui::new(&g.ctx.device, &g.ctx.queue, Some(&self.world.paths.repo_root), None, None);
                    u.set_screen(eo_ui::Screen::Hud);
                    u
                });
                ui.photo = Some(ph.panel(aspect));
                ui.set_time(self.process_start.elapsed().as_secs_f64());
                let _ = ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, g.config.width, g.config.height, &eo_ui::Input::default());
                ui.draw(&g.ctx.device, &mut enc, &tv, g.config.format, None);
            }
            eo_render::pace::submit(&g.ctx.queue, [enc.finish()]); // counted
            g.window.pre_present_notify();
            g.ctx.queue.present(surf);
            if let Some(ph) = self.photo.as_mut() {
                if let Some(scale) = ph.request.take() {
                    let r = crate::photo::capture(&g.ctx, &g.renderer, &cam, (g.config.width, g.config.height), g.config.format, scale, ph.describe(&cam));
                    match r {
                        Ok(p) => ph.say(format!("Saved {}", p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())),
                        Err(e) => ph.say(format!("Not saved: {e:#}")),
                    }
                }
            }
            self.fps_frames += 1;
            let el = self.fps_t.elapsed().as_secs_f64();
            if el >= 1.0 {
                let (mut mode, speed) = match self.player.mode {
                    Mode::OnFoot => ("on foot", self.player.ground_speed),
                    Mode::Driving(i) => ("driving", self.player.vehicles[i].forward_speed(&self.cw).abs()),
                };
                if self.car_access.is_active() {
                    mode = if self.player.mode == Mode::OnFoot { "entering car" } else { "exiting car" };
                }
                let door_hint = self
                    .door
                    .as_ref()
                    .filter(|d| self.player.mode == Mode::OnFoot && d.in_reach(pos))
                    .map_or("", |d| d.action_hint());
                let bed_hint = if self.player.mode == Mode::OnFoot && self.world.house.as_ref().is_some_and(|h| h.by_bed(pos)) { " - R: sleep until 07:00" } else { "" };
                // a landmark lift's stop: the prompt leads the title (the play window has no HUD; none in photo mode).
                let prompt = self.player.lift_prompt(&self.cw).filter(|_| !photo_on).map(|p| format!("E: {} - ", crate::fade::prompt_text(&p))).unwrap_or_default();
                g.window.set_title(&format!(
                    "{prompt}Earth Online native - play - {:.0} fps - {mode} {:.1} m/s - player {:.2} ms avg / {:.2} max - F: car, V: new car, E: lift, P: photo mode, Esc: mouse{door_hint}{bed_hint}",
                    self.fps_frames as f64 / el,
                    speed,
                    self.player_ms_sum / self.fps_frames.max(1) as f64,
                    self.player_ms_max
                ));
                self.fps_frames = 0;
                self.player_ms_max = 0.0;
                self.player_ms_sum = 0.0;
                self.fps_t = Instant::now();
            }
        }

        /// P: enter or leave photo mode.
        fn toggle_photo(&mut self) {
            let (Some(ph), Some(g)) = (self.photo.as_mut(), self.gfx.as_ref()) else { return };
            if ph.on {
                if let Some(h) = ph.leave() {
                    g.renderer.look.set_time_of_day(h); // the game's hour again
                }
                g.renderer.photo_mode.set(None);
            } else {
                let aspect = g.config.width as f64 / g.config.height.max(1) as f64;
                // the game's view (in the house, the eye camera) and its hour (slept or not).
                let view = player_view(&mut self.player, &self.cw, &self.world, self.interior_eye, 0.0, aspect);
                let cam = to_camera(&view, aspect as f32, self.args.far);
                ph.game_hour = g.renderer.look.time_of_day();
                ph.enter(&cam, crate::photo::anchor_of(self.player.position(&self.cw)));
                self.raw.held.clear();
                self.interact_requested = false;
                self.vehicle_toggle_requested = false;
            }
        }

        fn spawn_car_ahead(&mut self) {
            if self.player.mode != Mode::OnFoot || self.car_access.is_active() {
                return;
            }
            let f = self.player.character.feet();
            let y = self.player.yaw;
            self.player.spawn_vehicle(&mut self.cw, f.x + y.sin() * 6.0, f.z + y.cos() * 6.0, y);
        }
    }

    fn mouse_code(b: MouseButton) -> Option<&'static str> {
        match b {
            MouseButton::Left => Some("Mouse0"),
            MouseButton::Middle => Some("Mouse1"),
            MouseButton::Right => Some("Mouse2"),
            _ => None,
        }
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            if self.gfx.is_none() {
                if let Err(e) = self.init(el) {
                    eprintln!("init failed: {e:#}");
                    el.exit();
                }
            }
        }

        fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => el.exit(),
                WindowEvent::Resized(s) => {
                    if let Some(g) = self.gfx.as_mut() {
                        if s.width > 0 && s.height > 0 {
                            g.config.width = s.width;
                            g.config.height = s.height;
                            g.surface.configure(&g.ctx.device, &g.config);
                            g.targets = Targets::new(&g.ctx.device, s.width, s.height, g.config.format, self.args.msaa);
                        }
                    }
                }
                WindowEvent::Focused(false) => {
                    self.raw.held = HashSet::new();
                    self.interact_requested = false;
                    self.vehicle_toggle_requested = false;
                    self.set_looking(false);
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    if state == ElementState::Pressed && !self.looking {
                        self.set_looking(true);
                        return;
                    }
                    if let Some(c) = mouse_code(button) {
                        match state {
                            ElementState::Pressed => {
                                self.raw.held.insert(c.to_string());
                            }
                            ElementState::Released => {
                                self.raw.held.remove(c);
                            }
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        let name = format!("{code:?}");
                        match event.state {
                            ElementState::Pressed => {
                                let photo_on = self.photo.as_ref().is_some_and(|p| p.on);
                                if (code == crate::photo::TOGGLE && !event.repeat && self.photo.is_some()) || (code == KeyCode::Escape && photo_on) {
                                    self.toggle_photo();
                                    return;
                                }
                                if photo_on && self.photo.as_mut().is_some_and(|p| p.key(code)) {
                                    return;
                                }
                                // A quick tap may start and finish between redraws.
                                // (in photo mode E flies the camera up: no interaction)
                                if code == KeyCode::KeyE && !event.repeat && !photo_on {
                                    self.interact_requested = true;
                                }
                                // (in photo mode F belongs to the photo panel: no car)
                                if code == KeyCode::KeyF && !event.repeat && self.car_door_animation && !photo_on {
                                    self.vehicle_toggle_requested = true;
                                }
                                if code == KeyCode::Escape {
                                    if self.looking {
                                        self.set_looking(false);
                                    } else {
                                        el.exit();
                                    }
                                    return;
                                }
                                // (photo mode holds the player and the hour: no sleeping; nor while getting in or out of the car)
                                if code == KeyCode::KeyR && !event.repeat && self.player.mode == Mode::OnFoot && !photo_on && !self.car_access.is_active() {
                                    if let (Some(house), Some(g)) = (&self.world.house, &self.gfx) {
                                        if house.by_bed(self.player.character.feet()) {
                                            g.renderer.look.set_time_of_day(7.0);
                                            self.player.character.teleport_feet(house.wake_position());
                                        }
                                    }
                                }
                                if code == KeyCode::KeyV && !event.repeat && !photo_on {
                                    self.spawn_car_ahead();
                                }
                                self.raw.held.insert(name);
                            }
                            ElementState::Released => {
                                self.raw.held.remove(&name);
                            }
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    if let Some(ph) = self.photo.as_mut().filter(|p| p.on) {
                        let n = match delta {
                            winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
                            winit::event::MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                        };
                        ph.wheel(n);
                    }
                }
                WindowEvent::RedrawRequested => self.redraw(),
                _ => {}
            }
        }

        fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
            if let DeviceEvent::MouseMotion { delta } = event {
                if self.looking {
                    self.raw.mouse_dx += delta.0;
                    self.raw.mouse_dy += delta.1;
                }
            }
        }

        fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod house_tests {
    use super::*;

    /// The tests that load the whole world take turns. Each load holds several
    /// GB and spawns its own worker threads; run in parallel (the test
    /// harness's default), a dozen of them at once drove the machine into a
    /// kernel spinlock-timeout panic (2026-11-21, panic-full reports naming
    /// this test binary, four reboots). A poisoned lock (a failed test) still
    /// lets the next test run.
    fn world_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
    #[test]
    fn beaver_street_to_bed_and_top_floor() {
        check_beaver_walk(false, false, false, false, false);
    }

    #[test]
    fn furnished_variant_reaches_bed_and_top_floor() {
        check_beaver_walk(true, false, false, false, false);
    }

    #[test]
    fn weathered_variant_keeps_the_entry_and_stairs_clear() {
        check_beaver_walk(true, true, false, false, false);
    }

    #[test]
    fn sewn_furniture_keeps_the_door_and_upstairs_route_clear() {
        check_beaver_walk(true, true, true, false, false);
    }

    #[test]
    fn photo_terrace_keeps_the_door_bed_and_stairs_reachable() {
        check_beaver_walk(true, true, true, true, false);
    }

    #[test]
    fn blender_bedroom_reads_its_cut_pattern_and_directional_light() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{BEDROOM_LIGHT_BIT, CUT_COTTON_BIT};
        let (args, _) = parse(&["--house-blender-bedroom".to_owned()]).unwrap();
        let mut args = args;
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let cotton = world.scene.objects.iter().find(|p| p.kind & CUT_COTTON_BIT != 0)
            .expect("expected the cut cotton surface");
        assert_ne!(cotton.kind & BEDROOM_LIGHT_BIT, 0);
        let start = cotton.first_index as usize;
        let coords: std::collections::BTreeSet<_> = world.scene.indices[start..start+cotton.index_count as usize]
            .iter().map(|&i| world.scene.vertices[(cotton.base_vertex as i64+i64::from(i)) as usize].color >> 16)
            .collect();
        assert!(coords.len() >= 8_000, "the cut coordinates must distinguish points across the cloth");
        assert!(coords.contains(&0) && coords.contains(&0xffff));
        let (off, _) = parse(&["--house-blender-bedroom".to_owned(), "--no-house-blender-bedroom".to_owned()]).unwrap();
        assert!(off.house_photo && !off.house_blender_bedroom);
    }

    #[test]
    fn dining_light_registers_the_two_pages_on_the_installed_floor() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{DINING_FLOOR_LIGHT_BIT, FLOOR_SCAN_BIT};
        let (mut args, _) = parse(&["--house-dining-light".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let floor = world.house.as_ref().unwrap().point(0.,0.,0.).y as f32;
        let boards: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & DINING_FLOOR_LIGHT_BIT != 0).collect();
        assert!(!boards.is_empty());
        assert!(boards.iter().all(|o| o.kind & FLOOR_SCAN_BIT != 0 && o.min[1] <= floor + 0.04));
        let maps = crate::house::Finishes::load(&world.paths.repo_root, &args).unwrap();
        let lamp = &maps.yard_reflections.as_ref().unwrap().orm;
        let sky = &maps.atlas.orm;
        let res = eo_assets::kit::HIGH_RES;
        assert_eq!((lamp.len(),sky.len()),(4*res*res,4*res*res));
        // The neutral floor receives direct lamp light beside the table. The
        // tabletop blocks it at the centre. The kernel's origin is the street.
        let texel = |page: &[u8], x: f32, z: f32, channel: usize| {
            page[4*((z*res as f32/13.) as usize*res+(x*res as f32/7.) as usize)+channel]
        };
        assert!(texel(lamp,2.55,5.9,0) > texel(lamp,2.55,7.1,0));
        assert!(sky.chunks_exact(4).any(|t| t[0] > 5 && t[3] > 5));
        let (off, _) = parse(&["--house-dining-light".into(),"--no-house-dining-light".into()]).unwrap();
        assert!(off.house_photo && !off.house_dining_light);
        let original = crate::house::Finishes::load(&world.paths.repo_root, &off).unwrap();
        assert!(original.atlas.orm.iter().all(|&b| b==0));
        assert!(original.yard_reflections.unwrap().orm.iter().all(|&b| b==0));
    }

    #[test]
    fn table_survey_lights_the_board_and_place_settings_independently_of_the_floor() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{DINING_FLOOR_LIGHT_BIT, TABLE_BOARD_BIT, TABLE_LIGHT_BIT};
        let (mut args, _) = parse(&["--house-table-light".into()]).unwrap();
        assert!(args.house_photo && !args.house_downstairs_light && !args.house_dining_light);
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let marked: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & TABLE_LIGHT_BIT != 0).collect();
        assert_eq!(marked.iter().filter(|o| o.kind & TABLE_BOARD_BIT != 0).count(), 4);
        // Cups and cabinetry share the porcelain draw. Its bounds extend
        // well beyond the table although some of its vertices sit on the wood.
        assert!(marked.iter().any(|o| (glam::Vec3::from(o.max) - glam::Vec3::from(o.min)).length() > 4.0));
        assert!(world.scene.objects.iter().all(|o| o.kind & DINING_FLOOR_LIGHT_BIT == 0));
        let house = world.house.as_ref().unwrap();
        assert!((house.table_light()[0][3] - 0.75).abs() < 0.0001);
        assert!(house.dining_lamp().is_some());

        args.house_downstairs_light = true;
        let atlas = |args: &Args| crate::house::Finishes::load(&world.paths.repo_root, args)
            .unwrap().street_reflections.unwrap().orm;
        let on = atlas(&args);
        args.house_table_light = false;
        let off = atlas(&args);
        let mut changed = 0;
        for (i, (a,b)) in on.chunks_exact(4).zip(off.chunks_exact(4)).enumerate() {
            if a != b {
                assert!(i % 512 >= 256 && i / 512 < 256);
                changed += 1;
            }
        }
        assert!(changed > 4096);
        let (disabled, _) = parse(&["--house-table-light".into(), "--no-house-table-light".into()]).unwrap();
        assert!(!disabled.house_table_light);
        args.house_table_light = true;
        args.interior_lighting = false;
        assert!(atlas(&args).iter().all(|&c| c == 0));
    }

    #[test]
    fn cut_face_material_and_edge_meters_preserve_the_place_settings_and_roughness() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{TABLE_BOARD_BIT, TABLE_LIGHT_BIT};
        let (mut args, _) = parse(&["--house-oak-finish".into(), "--house-table-light".into(),
            "--house-oak-joinery".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let house = world.house.as_ref().unwrap();
        let light = house.table_light();
        assert!((0.025..0.085).contains(&light[1][3]));
        let boards: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & TABLE_BOARD_BIT != 0).collect();
        assert_eq!(boards.len(),4);
        assert!(boards.iter().all(|o| o.kind & TABLE_LIGHT_BIT != 0));
        let root = &world.paths.repo_root;
        let with = crate::house::Finishes::load(root, &args).unwrap();
        args.house_oak_joinery = false;
        let prior = crate::house::Finishes::load(root, &args).unwrap();
        let row = 512*4;
        assert_eq!(&with.floorboards.orm[..416*row], &prior.floorboards.orm[..416*row]);
        assert!(with.floorboards.orm.chunks_exact(4).zip(prior.floorboards.orm.chunks_exact(4))
            .all(|(a,b)| a[1] == b[1]));
        assert_ne!(&with.floorboards.orm[416*row..], &prior.floorboards.orm[416*row..]);
        let a = &with.street_reflections.as_ref().unwrap().orm;
        let b = &prior.street_reflections.as_ref().unwrap().orm;
        assert_eq!(&a[192*row..], &b[192*row..]);
        assert_eq!(&a[..26*row], &b[..26*row]);
        assert_eq!(&a[96*row..122*row], &b[96*row..122*row]);
        let (off, _) = parse(&["--house-oak-joinery".into(),"--no-house-oak-joinery".into()]).unwrap();
        assert!(!off.house_oak_joinery && !off.house_table_light);
    }

    #[test]
    fn ground_floor_reflections_upload_the_survey_on_its_four_board_materials() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{FLOOR_REFLECTION_BIT, WAXED_BOARD_BIT};
        let (mut args, _) = parse(&["--house-floor-reflections".into(), "--house-oak-finish".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let boards: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & FLOOR_REFLECTION_BIT != 0).collect();
        assert_eq!(boards.len(), 4);
        let floor = world.house.as_ref().unwrap().point(0.,0.,0.).y as f32;
        for o in boards {
            assert_ne!(o.kind & WAXED_BOARD_BIT, 0);
            assert!(o.min[1] <= floor && o.max[1] >= floor);
            let start = o.first_index as usize;
            let cells = &world.scene.indices[start..start + o.index_count as usize];
            let mut rows = std::collections::BTreeSet::new();
            for &i in cells {
                let vertex = world.scene.vertices[(o.base_vertex as i64 + i64::from(i)) as usize];
                if (vertex.pos[1] - floor).abs() > 0.04 { continue; }
                let v = (vertex.color >> 16) as f32 / 65535.;
                let strip = 8. * v;
                assert!((0.06..=0.94).contains(&strip.fract()));
                rows.insert(strip as usize);
            }
            assert_eq!(rows.len(), 8);
        }
        let root = &world.paths.repo_root;
        let maps = crate::house::Finishes::load(root, &args).unwrap().into_maps();
        assert_eq!(world.house_layer_count as usize, maps.len());
        let survey = &maps[maps.len()-5].orm;
        assert!(survey.chunks_exact(4).any(|p| p[2] > p[0] && p[2] > 100));
        args.house_floor_reflections = false;
        let previous = crate::house::Finishes::load(root, &args).unwrap().into_maps();
        assert_eq!(previous.len()+1, maps.len());
        for (a, b) in maps.iter().rev().take(4).zip(previous.iter().rev()) {
            assert_eq!(a.orm, b.orm);
        }
        let (disabled, _) = parse(&["--house-floor-reflections".into(),"--no-house-floor-reflections".into()]).unwrap();
        assert!(!disabled.house_floor_reflections && disabled.house_photo);
    }

    #[test]
    fn oak_reflections_keep_the_original_roughness_and_only_mark_the_four_boards() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{OAK_FINISH_BIT, TABLE_BOARD_BIT};
        let (mut args, _) = parse(&["--house-oak-finish".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        assert!(args.house_photo && !args.house_table_light && !args.house_dining_light);
        let world = load_world(&args).unwrap();
        let oak: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & OAK_FINISH_BIT != 0).collect();
        assert_eq!(oak.len(), 4);
        assert!(oak.iter().all(|o| o.kind & TABLE_BOARD_BIT != 0));
        let [bounds, material, extent] = world.house.as_ref().unwrap().oak_finish();
        assert!(extent[0] < bounds[0] && extent[3] > bounds[3]);
        assert!(bounds[2] > bounds[0] && bounds[3] > bounds[1]);
        for (measured, expected) in material.into_iter().zip([1., 0.15, 0.36, 0.04]) {
            assert!((measured-expected).abs() < 0.0001);
        }
        let atlas = |args: &Args| crate::house::Finishes::load(&world.paths.repo_root, args).unwrap().floorboards.orm;
        let on = atlas(&args);
        let (disabled, _) = parse(&["--house-oak-finish".into(), "--no-house-oak-finish".into()]).unwrap();
        assert!(!disabled.house_oak_finish);
        let old = atlas(&disabled);
        assert!(on.chunks_exact(4).zip(old.chunks_exact(4)).all(|(a,b)| a[1] == b[1]));
        assert!(on.chunks_exact(4).take(512*224).filter(|p| p[0] > 8).count() > 4096);
        args.interior_lighting = false;
        assert_eq!(atlas(&args), old);
    }

    #[test]
    fn furnished_floors_share_separate_rows_in_the_light_atlas() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::{DOWNSTAIRS_LIGHT_BIT, ENCLOSURE_TRANSFER_BIT};
        let (mut args, _) = parse(&["--house-downstairs-light".into(), "--house-blender-bedroom".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let mut marked = world.scene.objects.iter().filter(|o| o.kind & DOWNSTAIRS_LIGHT_BIT != 0).peekable();
        assert!(marked.peek().is_some());
        assert!(marked.all(|o| o.kind & ENCLOSURE_TRANSFER_BIT != 0));
        let rows = |args: &Args| {
            crate::house::Finishes::load(&world.paths.repo_root, args).unwrap()
                .street_reflections.unwrap().orm
        };
        let both = rows(&args);
        let half = both.len() / 2;
        args.house_downstairs_light = false;
        let upstairs = rows(&args);
        assert_eq!(&both[..half], &upstairs[..half]);
        assert!(both[half..].chunks_exact(4).any(|c| c[0] > 8 && c[1] > 8));
        assert!(!upstairs[half..].chunks_exact(4).any(|c| c[..3] != [0; 3]));
        args.house_downstairs_light = true;
        args.house_blender_bedroom = false;
        let ground = rows(&args);
        assert_eq!(&both[half..], &ground[half..]);
        assert!(ground[..half].iter().all(|&c| c == 0));
        args.interior_lighting = false;
        assert!(rows(&args).iter().all(|&c| c == 0));
        let (off, _) = parse(&["--house-downstairs-light".into(), "--no-house-downstairs-light".into()]).unwrap();
        assert!(off.house_photo && !off.house_downstairs_light);
    }

    #[test]
    fn mission_roof_replaces_only_the_high_clay_and_packs_its_firing_patches() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        use eo_render::scene::LOCAL_UV_SCAN_BIT;
        let (mut args, _) = parse(&["--house-mission-roof".into()]).unwrap();
        args.kit = false;
        args.models = false;
        args.landmarks = false;
        let world = load_world(&args).unwrap();
        let floor = world.house.as_ref().unwrap().point(0., 0., 0.).y as f32;
        let names = &world.authored.materials.names;
        let roof = world.scene.objects.iter().find(|o|
            names.get(o.material as usize).is_some_and(|s| s.ends_with("/mission-roof-clay"))
        ).expect("expected the installed clay roof");
        assert!(roof.min[1] > floor + 9.8 && roof.max[1] < floor + 11.5);
        assert_ne!(roof.kind & LOCAL_UV_SCAN_BIT, 0);
        let start = roof.first_index as usize;
        let cells: std::collections::BTreeSet<_> = world.scene.indices[start..start+roof.index_count as usize]
            .iter().map(|&i| {
                let v = world.scene.vertices[(roof.base_vertex as i64+i64::from(i)) as usize];
                ((v.color >> 22) & 3, v.color >> 30)
            }).collect();
        assert_eq!(cells.len(), 16, "all fired clay samples must reach the renderer");
        let old: Vec<_> = world.scene.objects.iter().filter(|o|
            names.get(o.material as usize).is_some_and(|s| s.contains("/fired-clay@"))
        ).collect();
        assert!(!old.is_empty() && old.iter().all(|o| o.max[1] < floor + 4.0),
                "the original porch awning must survive on its own");
        let (disabled, _) = parse(&["--house-mission-roof".into(), "--no-house-mission-roof".into()]).unwrap();
        assert!(disabled.house_photo && !disabled.house_mission_roof);
    }

    #[test]
    fn fitted_dining_is_walkable_from_the_closed_door_to_the_bed_and_kitchen() {
        check_beaver_walk(true, true, true, true, true);
        let (off, _) = parse(&["--house-fitted-dining".into(),"--no-house-fitted-dining".into()]).unwrap();
        assert!(off.house_photo && !off.house_fitted_dining);
    }

    #[test]
    fn fired_stoop_follows_installed_grade_with_its_own_materials() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for enabled in [false, true] {
            let mut flags=vec!["--house-fired-stoop".into()];
            if !enabled { flags.push("--no-house-fired-stoop".into()); }
            let (mut args,_)=parse(&flags).unwrap();
            args.kit=false;
            args.models=false;
            args.agents=false;
            let world=load_world(&args).unwrap();
            let house=world.house.as_ref().unwrap();
            let table=&world.authored.materials;
            let installed: Vec<_>=world.scene.objects.iter().filter(|o|
                table.names[o.material as usize].starts_with("21-beaver-fitted/Entrance/")).collect();
            assert_eq!(installed.is_empty(), !enabled);
            if enabled {
                let materials: std::collections::BTreeSet<_> = installed.iter().map(|o|o.material).collect();
                assert_eq!(materials.len(),2);
                assert!(materials.iter().all(|s|table.gpu[*s as usize].layer>=0));
                assert!(installed.iter().all(|o|o.kind & eo_render::scene::FITTED_ROOM_BIT != 0));
                let (_, door) = collision_world(&world, 2, false).unwrap();
                assert!(door.unwrap().is_shut());
                let origin = house.point(5.95,0.,0.);
                let depths: Vec<_>=installed.iter().map(|o|o.min[1] as f64-origin.y).collect();
                assert!(depths.iter().any(|h|h.abs()<0.04) && depths.iter().any(|h|*h < -0.5));
            }
        }
    }

    #[test]
    fn porch_switch_uses_the_emitting_shade_and_its_own_atlas_layer() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (walls, enabled) in [(false, true), (true, true), (true, false)] {
            let mut flags = ["--house-porch-lamp", "--house-street-garden", "--no-interior-lighting"]
                .map(str::to_owned).to_vec();
            if walls { flags.push("--house-wall-daylight".into()); }
            if !enabled { flags.push("--no-house-porch-lamp".into()); }
            let (mut args, _) = parse(&flags).unwrap();
            args.kit = false;
            args.models = false;
            args.agents = false;
            args.props = false;
            let world = load_world(&args).unwrap();
            let house = world.house.as_ref().unwrap();
            let table = &world.authored.materials;
            let light = house.porch_light(table.layers.len());
            let shade = table.names.iter().position(|n| n == "21-beaver/opal-porch-lamp");
            assert_eq!(light.is_some(), enabled);
            assert_eq!(shade.is_some(), enabled);
            if let (Some(field), Some(id)) = (light, shade) {
                let [r,g,b,_] = table.gpu[id].emissive;
                assert!(r > g && g > b && b > 1.);
                assert!(world.scene.objects.iter().any(|o| o.material == id as u32));
                let slot = field[1][3] as usize;
                if let Some(wall) = house.wall_daylight_layer {
                    assert_eq!(table.layers.len() + wall + 1, slot);
                }
                assert!(table.names.iter().zip(&table.gpu)
                    .filter(|(n,_)| n.starts_with("21-beaver-fitted/Street garden/"))
                    .all(|(_, material)| material.layer > slot as i32));
                assert!(field[5][3] > 2. && field[6][2] > 0.75);
                assert_eq!(field[6][3], 0.);
            }
        }
    }

    #[test]
    fn window_scene_follows_the_photo_house_and_glazing_switches() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (flags, expected) in [
            (vec!["--house-window-optics"], true),
            (vec!["--house-photo", "--house-window-optics", "--no-house-glazing"], false),
            (vec!["--house-photo", "--house-window-optics", "--no-house-window-optics"], false),
            (vec!["--house-joinery-light", "--no-house-glazing"], true),
            (vec!["--house-joinery-light", "--no-house-joinery-light"], false),
            (vec!["--house-window-optics", "--house-joinery-light", "--no-house-window-optics"], true),
        ] {
            let (mut args, _) = parse(&flags.iter().map(|v| v.to_string()).collect::<Vec<_>>()).unwrap();
            args.kit = false;
            args.models = false;
            args.agents = false;
            args.props = false;
            let world = load_world(&args).unwrap();
            assert_eq!(world.house.unwrap().window_optics.is_some(), expected);
        }
    }

    #[test]
    fn plaster_survey_keeps_the_fitted_atlases_and_camera_independent() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (walls, camera) in [(true,false),(false,true),(true,true),(false,false)] {
            let mut flags = ["--house-wall-daylight","--house-site-camera","--blender-camera",
                "--house-front-daylight","--house-fitted-dining","--no-interior-lighting"]
                .map(str::to_owned).to_vec();
            if !walls { flags.push("--no-house-wall-daylight".into()); }
            if !camera { flags.push("--no-house-site-camera".into()); }
            let (mut args, _) = parse(&flags).unwrap();
            args.kit = false;
            args.models = false;
            args.agents = false;
            args.props = false;
            let world = load_world(&args).unwrap();
            let house = world.house.as_ref().unwrap();
            assert_eq!(house.site_camera().is_some(),camera);
            assert!(house.interior_meter().is_none());
            assert_eq!(house.wall_daylight_layer.is_some(),walls);
            let table = &world.authored.materials;
            let mut marked = 0;
            for o in &world.scene.objects {
                if o.kind & eo_render::scene::WALL_DAYLIGHT_BIT == 0 { continue; }
                marked += 1;
                let name = &table.names[o.material as usize];
                assert!(["painted-stucco@", "float-finish-stucco@", "lime-plaster@", "painted-white@"].iter().any(|f| name.contains(f)), "{name}");
            }
            assert_eq!(marked>0,walls);
            if let Some(slot) = house.wall_daylight_layer {
                let layer = (table.layers.len()+slot) as i32;
                assert!(table.names.iter().zip(&table.gpu).filter(|(n,_)| n.starts_with("21-beaver-fitted/Street garden/")).all(|(_,m)| m.layer>layer));
            }
        }
    }

    #[test]
    fn exterior_daylight_reaches_only_the_fitted_bed_and_survives_disabled_room_light() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (enabled, room_light) in [(false, true), (true, false)] {
            let mut flags = vec!["--house-front-daylight".to_owned()];
            if !enabled {
                flags.push("--no-house-front-daylight".into());
            }
            let (mut args, _) = parse(&flags).unwrap();
            assert!(args.house_street_garden);
            assert_eq!(args.house_front_daylight, enabled);
            args.kit = false;
            args.models = false;
            args.landmarks = false;
            args.interior_lighting = room_light;
            let world = load_world(&args).unwrap();
            let house = world.house.as_ref().unwrap();
            assert_eq!(house.front_daylight.is_some(), enabled);
            if !room_light {
                assert!(house.interior_meter().is_none());
            }
            let local = house.parcel_to_world().inverse().transform_point3(house.point(2.62, 0.4, -1.78).as_vec3());
            assert!(local.distance(glam::Vec3::new(2.62, 0.4, -1.78)) < 0.001);
            let table = &world.authored.materials;
            let mut marked = 0;
            for obj in &world.scene.objects {
                let idx = obj.material as usize;
                let surveyed = table.names[idx].starts_with("21-beaver-fitted/Street garden/");
                let receives = table.gpu[idx].flags & eo_render::authored::F_FRONT_DAYLIGHT != 0;
                assert_eq!(receives, surveyed && enabled);
                marked += usize::from(receives);
            }
            assert_eq!(marked, if enabled { 3 } else { 0 });
        }
    }

    #[test]
    fn street_garden_preserves_the_entry_and_the_fitted_room_maps() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        let common = [
            "--house-window-linen",
            "--house-fitted-finishes",
            "--house-floor-reflections",
            "--house-mission-roof",
        ];
        let mut control = None;
        for enabled in [false, true] {
            let mut flags = common.map(String::from).to_vec();
            flags.push("--house-street-garden".into());
            if !enabled {
                flags.push("--no-house-street-garden".into());
            }
            let (mut args, _) = parse(&flags).unwrap();
            assert_eq!(args.house_street_garden, enabled);
            args.kit = false;
            args.landmarks = false;
            args.models = false;
            let world = load_world(&args).unwrap();
            let house = world.house.as_ref().unwrap();
            let table = &world.authored.materials;
            let name = |o: &eo_render::scene::GpuObject| table.names[o.material as usize].as_str();
            let old = world
                .scene
                .objects
                .iter()
                .filter(|o| name(o).contains("agave-leaf@"))
                .count();
            assert_eq!(old > 0, !enabled);
            let garden: Vec<_> = world
                .scene
                .objects
                .iter()
                .filter(|o| name(o).starts_with("21-beaver-fitted/Street garden/"))
                .collect();
            assert_eq!(garden.len(), if enabled { 3 } else { 0 });
            let maps = crate::house::Finishes::load(&world.paths.repo_root, &args)
                .unwrap()
                .into_maps();
            let bound = |o: &eo_render::scene::GpuObject| {
                (table.gpu[o.material as usize].layer as usize) - table.layers.len()
            };
            let blind = world
                .scene
                .objects
                .iter()
                .find(|o| name(o).ends_with("/Sun-filtering plainweave flax"))
                .unwrap();
            let stable = (
                house.entry_position(),
                house.lowest_tread_corners(),
                maps[bound(blind)].albedo.clone(),
                maps[maps.len() - 6].albedo.clone(),
            );
            if let Some(prior) = &control {
                assert_eq!(&stable, prior);
            } else {
                control = Some(stable);
                continue;
            }
            for o in garden {
                assert_ne!(o.kind & eo_render::scene::FITTED_ROOM_BIT, 0);
                assert_eq!(o.kind & eo_render::scene::DOWNSTAIRS_LIGHT_BIT, 0);
                let page = &maps[bound(o)];
                assert!(
                    page.normal
                        .chunks_exact(4)
                        .any(|p| p[2] > 190 && p[1] != 128)
                );
                let begin = o.first_index as usize;
                let contact = world.scene.indices[begin..begin + o.index_count as usize]
                    .iter()
                    .map(|&index| {
                        (world.scene.vertices[(o.base_vertex as i64 + i64::from(index)) as usize]
                            .normal
                            >> 24) as i8
                    });
                assert!(contact.clone().any(|v| v < -25));
                assert!(contact.clone().any(|v| v > -5));
            }
        }
        let (standalone, _) = parse(&["--house-street-garden".into()]).unwrap();
        assert!(standalone.house_photo && !standalone.house_fitted_dining);
    }

    #[test]
    fn fitted_window_replaces_one_blind_and_binds_both_cloth_faces() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (lighting, enabled) in [(true, true), (false, true), (true, false)] {
            let mut flags = vec!["--house-window-linen".to_owned(), "--house-fitted-finishes".to_owned(), "--house-kitchen-light".to_owned()];
            if !enabled { flags.push("--no-house-window-linen".to_owned()); }
            let (mut args, _) = parse(&flags).unwrap();
            args.interior_lighting = lighting;
            args.kit = false;
            args.landmarks = false;
            args.models = false;
            let world = load_world(&args).unwrap();
            let table = &world.authored.materials;
            let material_name = |obj: &eo_render::scene::GpuObject| table.names[obj.material as usize].as_str();
            let old = world.scene.objects.iter().filter(|o| material_name(o).ends_with("/Unbleached translucent Roman blind")).count();
            let installed: Vec<_> = world.scene.objects.iter().filter(|o|material_name(o).starts_with("21-beaver-fitted/Window/")).collect();
            assert_eq!(old,usize::from(!enabled));
            assert_eq!(installed.len(),if enabled {4} else {0});
            if !enabled { continue; }
            let cloth = installed.iter().find(|o|material_name(o).ends_with("/Sun-filtering plainweave flax")).unwrap();
            let mat = table.gpu[cloth.material as usize];
            assert_eq!(mat.flags,0);
            assert_eq!(mat.thin_absorption,if lighting {-1.} else {0.});
            if lighting {
                let pages = crate::house::Finishes::load(&world.paths.repo_root,&args).unwrap().into_maps();
                let i = usize::try_from(mat.layer).unwrap().checked_sub(table.layers.len()).unwrap();
                let maps = &pages[i];
                // Each face carries eight different light sources, and the third
                // image is neutral-centred yarn detail. A kit layer is invalid here.
                assert_ne!(maps.albedo,maps.normal);
                assert!(maps.albedo.chunks_exact(maps.albedo.len()/8).all(|row|row.chunks_exact(4).any(|v| v[0] > 10)));
                assert!(mat.base[0] > 9. && mat.base[2] < -2. && mat.base[3] >= 0.99);
                let arclengths: std::collections::BTreeSet<_> = maps.orm.chunks_exact(4).map(|p| u16::from_be_bytes([p[1],p[2]])).collect();
                assert!(arclengths.len() > 1024);
            } else {
                assert_eq!(mat.layer,-1);
            }
        }
    }

    #[test]
    fn fitted_kitchen_light_binds_the_tile_wall_and_both_sides_of_the_pendant() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        for (switches, kitchen, dining) in [
            (vec!["--house-kitchen-light"], true, false),
            (vec!["--house-kitchen-light", "--house-fitted-finishes"], true, true),
            (vec!["--house-fitted-finishes", "--house-kitchen-light", "--no-house-kitchen-light"], false, true),
        ] {
            let flags: Vec<_> = switches.into_iter().map(String::from).collect();
            let (mut args, _) = parse(&flags).unwrap();
            assert!(args.house_fitted_dining);
            assert_eq!((args.house_kitchen_light, args.house_fitted_finishes), (kitchen, dining));
            args.kit = false;
            args.landmarks = false;
            args.models = false;
            let world = load_world(&args).unwrap();
            let table = &world.authored.materials;
            let material = |name: &str| {
                let name = format!("21-beaver-fitted/{name}");
                table.gpu[table.names.iter().position(|s| s == &name).unwrap()]
            };
            let tile = material("Warm white glazed kitchen tile 1");
            let shade = material("Black stove enamel");
            let lining = material("Cream vitreous reflector");
            let bowl = material("Fired sage ash glaze");
            assert_eq!(tile.thin_absorption, if kitchen { 8. } else { 0. });
            assert_eq!(shade.thin_absorption, if kitchen { 6. } else { 0. });
            assert_eq!(lining.thin_absorption, if kitchen { 7. } else { 0. });
            assert_eq!(bowl.thin_absorption, if dining { 2. } else { 0. });
            let frame = world.house.as_ref().unwrap().kitchen_frame();
            if kitchen {
                assert_eq!(tile.params[3], shade.params[3]);
                assert_eq!(tile.params[3], lining.params[3]);
                if dining { assert_eq!(tile.params[3], bowl.params[3] + 1.); }
                let [along, across, heights, low, high] = frame;
                assert!(low[0] < high[0]);
                let oak = material("Chopping board oak");
                assert_eq!(material("oiled-oak@ae906f").layer, -1);
                assert_eq!(oak.thin_absorption, 13.);
                assert_eq!(oak.layer, tile.params[3] as i32 + 1);
                assert_eq!(oak.base, [1.; 4]);
                assert!(along[2] < 0. && across[0] < 0. && heights[2] < heights[0]
                    && heights[0] < heights[1] && heights[1] < heights[3]);
            } else {
                assert_eq!(frame, [[0.; 4]; 5]);
            }
        }
    }

    #[test]
    fn fitted_finishes_bind_the_registered_coats_and_restore_the_prior_room() {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        let (mut args, _) = parse(&["--house-fitted-finishes".into()]).unwrap();
        assert!(args.house_fitted_dining && args.house_fitted_finishes);
        args.kit = false;
        args.landmarks = false;
        args.models = false;
        let world = load_world(&args).unwrap();
        let table = &world.authored.materials;
        let find = |name: &str| {
            let i = table.names.iter().position(|s| s == name).unwrap();
            table.gpu[i]
        };
        let glaze = find("21-beaver-fitted/Fired sage ash glaze");
        let rail = find("21-beaver-fitted/Waxed walnut stair handrail");
        assert_eq!(rail.thin_absorption, 1.);
        assert_eq!(glaze.thin_absorption, 2.);
        assert!((rail.emissive[3] - 0.24).abs() < 1e-5 && (rail.rough_flatten - 0.19).abs() < 1e-5);
        assert!((glaze.emissive[3] - 0.32).abs() < 1e-5);
        assert_eq!(rail.params[3], glaze.params[3]);
        let linen = find("21-beaver-fitted/Laundered flax with woven indigo borders");
        assert_eq!(linen.thin_absorption, 5.);
        assert_eq!(linen.params[3], glaze.params[3]);
        let [bounds, heights] = world.house.as_ref().unwrap().fitted_finish();
        assert!(
            bounds[0] < bounds[2]
                && bounds[1] < bounds[3]
                && heights[0] < heights[1]
                && heights[1] < heights[2]
        );
        let (off, _) = parse(&[
            "--house-fitted-finishes".into(),
            "--no-house-fitted-finishes".into(),
        ])
        .unwrap();
        assert!(off.house_fitted_dining && !off.house_fitted_finishes);
    }

    fn check_beaver_walk(finishes: bool, patina: bool, furniture: bool, photo: bool, fitted: bool) {
        let _world = world_lock(); // (one whole-world load at a time: see world_lock)
        let mut args = base_args();
        args.house_finishes = finishes;
        args.house_patina = patina;
        args.house_furniture = furniture;
        args.house_photo = photo;
        args.house_fitted_dining = fitted;
        args.house_fired_stoop = fitted;
        args.house_photo_lod = photo;
        args.beaver_house = true;
        args.kit = false;
        args.landmarks = false;
        args.models = false;
        let mut world = load_world(&args).unwrap();
        let house = world.house.as_ref().unwrap();
        if photo {
            use eo_render::scene::{LOD_AUTHORED_DETAIL, TABLE_BOARD_BIT, THIN_FIBRE_BIT};
            let parts = &world.scene.objects;
            let boards: std::collections::BTreeSet<_> = parts.iter()
                .filter(|p| if fitted {
                    p.kind & eo_render::scene::FITTED_ROOM_BIT != 0 && world.authored.materials.names[p.material as usize].starts_with("21-beaver-fitted/Dining tabletop · plank")
                } else {p.kind & TABLE_BOARD_BIT != 0}).map(|p| p.material).collect();
            assert_eq!(boards.len(), 4, "every dining plank retains its grain mapping");
            assert!(parts.iter().any(|p| {
                let centre = (Vec3::from(p.min) + Vec3::from(p.max)) * 0.5;
                let yard = house.point(3., 0., 26.).as_vec3();
                p.kind & THIN_FIBRE_BIT != 0 && (centre - yard).length() < 11. && p.index_count > 30
            }), "expected planted meshes at courtyard grade");
            let reduced: Vec<_> = parts.iter().filter(|p| p.lod_role == LOD_AUTHORED_DETAIL).collect();
            assert!(reduced.iter().all(|p| p.far_index_count < p.index_count));
            assert!(reduced.iter().map(|p| p.index_count - p.far_index_count).sum::<u32>() > 100_000);
        }
        if patina {
            use eo_render::scene::{BLEND_BIT, FRONT_SASH_BIT};
            let panes: Vec<_> = world.scene.objects.iter().filter(|o| o.kind & FRONT_SASH_BIT != 0).collect();
            assert!(!panes.is_empty(), "the front windows lost their reflection");
            assert!(panes.iter().all(|o| o.kind & BLEND_BIT != 0), "glass must still transmit the furnished rooms");
        }
        let (mut cw, door) = collision_world(&world, 2, false).unwrap();
        let mut door = door.unwrap();
        assert!(door.is_shut());
        if furniture {
            let floor = house.point(2., 0., 5.);
            let mut resident = Player::new(floor, 0.);
            assert!((2. * resident.character.half_extent_y() - 1.80).abs() < 1e-6);
            for (crouch, height) in [(false, 1.70), (true, 1.10)] {
                resident.character.set_crouched(&cw, crouch);
                let view = player_view(&mut resident, &cw, &world, true, 1. / 60., 16. / 9.);
                assert!((view.eye.y - floor.y - height).abs() < 1e-5 && !view.body_visible);
            }
            assert!(!house.contains_feet(house.entry_position()));
        }
        let mut start = house.entry_position();
        if finishes {
            assert!(
                start.distance(house.point(5.95, 0., -3.3)) > 0.01,
                "private path must reach the mapped paving"
            );
            let ground = &world.street.as_ref().unwrap().world;
            for corner in house.lowest_tread_corners() {
                assert!(
                    corner.y > ground.surface_at(corner.x, corner.z, None) + 0.03,
                    "the hillside cuts the lowest tread"
                );
            }
        }
        cw.load_blocking(start, pworld::LOAD_RADIUS_M);
        start.y = cw.surface_y(start.x, start.z);
        if furniture {
            let origin = house.point(0., 0., 0.);
            let x_axis = (house.point(1., 0., 0.) - origin).normalize();
            let z_axis = x_axis.cross(DVec3::Y) * x_axis.cross(DVec3::Y).dot(house.point(0., 0., 1.) - origin).signum();
            // Collision probes are measured from the installed centre of each table.
            let dining = if fitted { house.point(3.137,0.,6.537) } else { house.point(2.375, 0., 7.575) + if photo {
                x_axis * 0.17 - z_axis * 0.70
            } else { DVec3::ZERO } };
            let table_axis = if fitted {
                (house.point(3.281,0.,7.555) - dining).normalize()
            } else if photo { -x_axis } else { z_axis };
            let near_chair = if photo { 1.053 } else { 1.233 };
            let kitchen = house.point(0.15, 0., 12.72);
            for (label, surface, height) in [
                ("dining table", dining, 0.75),
                ("chair seat", dining - table_axis * near_chair, 0.45),
                ("far chair seat", dining + table_axis * 1.327, 0.45),
                ("kitchen counter", kitchen + x_axis * 0.4 - z_axis * 0.24, 0.92),
                ("fridge", kitchen + x_axis * 1.50 - z_axis * 0.36, 1.8952),
            ] {
                let above = surface + DVec3::Y * (height + 0.10);
                let hit = cw.raycast(above, DVec3::NEG_Y, 0.2, pworld::layer::WORLD, None)
                    .unwrap_or_else(|| panic!("{label} has no collision at its authored height"));
                assert!((hit.distance - 0.10).abs() < 0.003, "{label} collider is at the wrong height");
            }
        }
        // Use the play window's controller and keyboard mapping for the
        // approach, including its acceleration, slope and step handling.
        let inside = house.point(5.95, 0., 1.05);
        let mut player = Player::new(start, house.entry_yaw());
        let mut raw = RawInput::default();
        raw.held.insert("KeyW".to_owned());
        let mut actions = ActionMap::default();
        for _ in 0..360 {
            let frame = actions.resolve(&mut raw, 1. / 60., false);
            player.advance(&mut cw, &StepInput::from_frame(&frame), 1. / 60.);
        }
        let mut body = player.character;
        assert!(
            body.feet().distance(inside) > 1.0,
            "walked through the shut front door"
        );
        raw.held.clear();
        raw.held.insert("KeyE".to_owned());
        assert!(
            actions
                .resolve(&mut raw, 1. / 60., false)
                .tapped(eo_player::input::Action::Interact)
        );
        assert!(
            door.toggle(body.feet()),
            "cannot operate the front door from the stoop: {:?}",
            body.feet()
        );
        for _ in 0..35 {
            door.advance(1. / 60., body.feet(), &mut world.scene, &mut cw);
            cw.physics.step();
        }
        let outside = house.point(5.89, 1.03, -0.35);
        let ray = house.point(5.89, 1.03, 0.65) - outside;
        assert!(
            cw.raycast(
                outside,
                ray.normalize(),
                ray.length(),
                pworld::layer::WORLD,
                None
            )
            .is_none(),
            "open leaf still blocks the opening"
        );
        if furniture {
            // Measure the installed opening at the player's shoulders and above the lintel.
            let across = (house.point(1., 0., 0.) - house.point(0., 0., 0.)).normalize();
            let clear = |offset, height| {
                let start = house.point(5.95, height, -0.35) + across * offset;
                cw.raycast(start, ray.normalize(), ray.length(), pworld::layer::WORLD, None)
                    .is_none()
            };
            assert!(clear(-0.42, 1.4) && clear(0.42, 1.4), "shoulders catch the open door");
            assert!(!clear(-0.53, 1.4) && !clear(0.53, 1.4), "missing fixed side returns");
            assert!(clear(0., 2.03) && !clear(0., 2.11), "door head clearance changed");
            // A whole standing character fits on level ground outside the shut leaf.
            let approach = (house.point(0., 0., 1.) - house.point(0., 0., 0.)).normalize();
            let shoulder = house.point(5.95, 1.4, -0.32);
            let ground = house.point(5.95, 0., -0.32);
            for travel in [0., 0.35, 0.70, 0.96] {
                let above = ground + approach * travel + DVec3::Y * 0.10;
                let hit = cw.raycast(above, DVec3::NEG_Y, 0.13, pworld::layer::WORLD, None)
                    .expect("missing entrance landing surface");
                assert!((hit.distance - 0.10).abs() < 0.015, "step in front of the latch");
            }
            let jamb = cw.raycast(shoulder - across * 0.54, approach, 1.2, pworld::layer::WORLD, None)
                .expect("missing fixed wall beside the leaf");
            assert!(jamb.distance >= 0.84, "player cannot stand inside the sheltered landing");
        }
        let mut walk = |x, z| {
            let target = house.point(x, 0.0, z);
            for _ in 0..1200 {
                let p = body.feet();
                let delta = glam::DVec2::new(target.x - p.x, target.z - p.z);
                if delta.length() < 0.035 {
                    return p;
                }
                let d = delta.clamp_length_max(0.033);
                body.step(&cw, d.x, d.y, true);
            }
            panic!("blocked at {:?} walking to ({x}, {z})", body.feet());
        };
        if fitted {
            // Walk from the entrance to the table, along the kitchen, to the window bench
            // and back around the chairs to the stair. Each point is in the native parcel.
            for (x,z) in [(5.95,1.30),(4.55,2.60),(4.28,4.20),(1.80,4.20),
                (1.80,6.64),(1.35,9.38),(1.35,11.53),(3.50,11.53),(4.36,9.23),
                (4.45,7.62),(4.55,5.20),(4.50,3.65),(5.95,2.10)] {
                let feet = walk(x,z);
                assert!((feet.y-house.point(x,0.,z).y).abs()<0.06,"climbed furniture on the fitted route");
            }
        } else if furniture {
            // Walk both aisles between the sideboard, the six chairs, and the stairs.
            for (x, z) in [(5.95, 1.3), (4.10, 1.3), (4.10, 5.60), (1.00, 5.60),
                           (1.00, 9.35), (4.10, 9.35), (4.10, 2.10), (5.95, 2.10)] {
                let feet = walk(x, z);
                assert!((feet.y - house.point(x, 0., z).y).abs() < 0.06, "stepped onto furniture in dining aisle");
            }
        }
        walk(5.95, 9.8);
        walk(3.35, 9.8);
        assert!(house.by_bed(walk(3.35, 10.8)));
        walk(4.15, 10.8);
        walk(4.15, 2.1);
        walk(5.95, 2.1);
        let loft = walk(5.95, 9.8);
        assert!(loft.y > house.wake_position().y + 2.5);
        walk(5.95, 2.1);
        walk(4.15, 2.1);
        walk(4.15, 9.8);
        walk(5.95, 9.8);
        let street = walk(5.95, -3.3);
        assert!((street.y - start.y).abs() < 0.3);
        // The open knob is on the inside. A resident can close the door,
        // it then blocks the opening, and either side can operate it again.
        let resident = house.point(5.24, 0., 1.42);
        assert!(door.toggle(resident));
        let in_swing = house.point(5.91, 0., 0.93);
        for _ in 0..12 {
            door.advance(0.1, in_swing, &mut world.scene, &mut cw);
            cw.physics.step();
        }
        assert!(
            !door.is_shut(),
            "the door closed through a person standing in its swing"
        );
        for _ in 0..35 {
            door.advance(1. / 60., resident, &mut world.scene, &mut cw);
            cw.physics.step();
        }
        assert!(door.is_shut());
        assert!(
            cw.raycast(
                outside,
                ray.normalize(),
                ray.length(),
                pworld::layer::WORLD,
                None
            )
            .is_some(),
            "closed leaf no longer blocks the opening"
        );
        assert!(door.toggle(resident));
        assert!(
            !door.toggle(house.point(2., 6.7, 8.)),
            "door can be operated from another floor"
        );
    }
}
