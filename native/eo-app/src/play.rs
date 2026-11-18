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
//! (agents) The player's body (the first pool person, hidden when the camera rig
//! says `body_visible = false` and while driving) and the player's cars are
//! drawn by the agent renderer, with the simulated traffic and people around
//! the player (`--no-agents`: none of it).
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
        bench: false,
        frames: 720,
        warmup: 60,
        width: 1920,
        height: 1080,
        msaa: 1,
        indirect_validation: true,
        no_indirect_count: false,
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
        props: true, // props
        props2: Default::default(),
        perf: Default::default(), // occlusion / hitch
        landmarks: true,          // landmarks
        landmarks2: true,
    }
}

fn parse(raw: &[String]) -> Result<(Args, PlayOpts)> {
    let mut a = base_args();
    let mut o = PlayOpts { bench: false, frames: None, segments: None, render: true, collision_threads: 2 };
    let mut it = raw.iter();
    while let Some(s) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| anyhow!("{s} needs a value"));
        match s.as_str() {
            "--play" => {}
            "--bench" => o.bench = true,
            "--frames" => o.frames = Some(val()?.parse()?),
            "--segments" => o.segments = Some(val()?.split(',').map(|s| s.trim().to_string()).collect()),
            "--no-render" => o.render = false,
            "--collision-threads" => o.collision_threads = val()?.parse::<usize>()?.max(1),
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
            "--no-rocketbox" => a.rocketbox = false, // people
            "--no-props" => a.props = false, // props
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
            "--no-external" => {} // main's `external_flag`
            other => anyhow::bail!("--play: unknown argument {other} (see native/eo-app/src/play.rs)"),
        }
    }
    a.bench = o.bench;
    Ok((a, o))
}

/// The collision world over the loaded city: every footprint as its JS prism
/// (lifted by the construction lift the renderer uses), standing on the drawn
/// street surface (eo-street: carpet, carriageway, kerbs, sidewalks) when the
/// streets are loaded, else on the elevation grid.
fn collision_world(world: &World, threads: usize) -> Result<CollisionWorld> {
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let buildings = Arc::new(BuildingSet::from_city(&world.city, lift));
    let ground: Arc<dyn GroundProvider> = match &world.street {
        Some(s) => Arc::new(StreetGround(s.world.clone())),
        None => Arc::new(ElevationGround(Arc::new(eo_data::ElevationGrid::load(&world.paths)?))),
    };
    Ok(CollisionWorld::new(ground, buildings, threads))
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
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
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
    let gpu_opts = GpuOptions { indirect_validation: args.indirect_validation, force_no_indirect_count: args.no_indirect_count, ..Default::default() };
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
    for name in ["walk_player", "drive_player"] {
        if let Some(only) = &opts.segments {
            if !only.iter().any(|s| s == name) {
                continue;
            }
        }
        let driving = name == "drive_player";
        let (path, steep_s) = uphill_route(&walk.path, &world.elevation);
        // Start far enough before the steepest stretch that the path crosses
        // several blocks on the level and then climbs it.
        let lead = if driving { 500.0 } else { 450.0 };
        let frames = opts.frames.unwrap_or(if driving { 3600 } else { 7200 });
        let path = from_arc(&path, (steep_s - lead).max(0.0));
        let mut cw = collision_world(&world, opts.collision_threads)?;
        let mut start = DVec3::new(path[0][0], world.elevation.ground(path[0][0], path[0][1]), path[0][1]);
        let t_spawn = Instant::now();
        cw.load_blocking(start, pworld::LOAD_RADIUS_M);
        let spawn_load_ms = t_spawn.elapsed().as_secs_f64() * 1e3;
        start.y = cw.surface_y(start.x, start.z);
        let yaw0 = (path[1][0] - path[0][0]).atan2(path[1][1] - path[0][1]);
        let mut player = Player::new(start, yaw0);
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
        let first = to_camera(&player.camera(&cw, dt, aspect as f64), aspect, args.far);
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
            }
        }
        let inserted0 = cw.stats.tiles_inserted;
        cw.stats.pump_ms_max = 0.0;
        let mut player_ms = Vec::with_capacity(frames);
        // how long the thread actually ran during the step, and the rest.
        let mut player_ran = Vec::with_capacity(frames);
        let mut player_off_cpu = Vec::with_capacity(frames);
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
            let input = if driving { drive_pilot.input(&player, &cw) } else { walk_pilot.input(&mut player, &cw) };
            player.advance(&mut cw, &input, dt);
            let pos = player.position(&cw);
            let tp = Instant::now();
            cw.pump(pos);
            let tc = Instant::now();
            let view = player.camera(&cw, dt, aspect as f64);
            let t1 = Instant::now();
            let ran = eo_render::hitch::on_cpu_ms(c0, eo_render::hitch::thread_cycles()); // hitch
            player_off_cpu.push(((t1 - t0).as_secs_f64() * 1e3 - ran).max(0.0));
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
            if let Some((ctx, renderer, _c, tv, targets, models)) = &mut gpu_state {
                let cam = to_camera(&view, aspect, args.far);
                models.set_camera(cam.eye);
                models.pump(ctx, renderer, &world, cam.eye)?;
                if let (Some(p), Some(rt)) = (props_rt.as_mut(), agents_rt.as_mut()) {
                    // tiles, lamps on the simulation clock, parked cars to the agents.
                    p.pump(ctx, renderer, &mut models.uploads, cam.eye, Some(rt.sim_time), dt);
                    rt.parked = p.parked_agents(rt.lib.as_deref(), cam.eye).to_vec();
                }
                if let Some(rt) = agents_rt.as_mut() {
                    // agents
                    rt.set_player(&mut visual, &player, &cw, view.body_visible, dt);
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
            }
            frame_cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3 + charged_ms);
            if (driving && drive_pilot.done()) || (!driving && walk_pilot.done()) {
                break;
            }
        }
        if let Some((ctx, ..)) = &gpu_state {
            ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
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
        println!(
            "  hitch: longest step frame {worst}: {:.3} ms wall, thread ran {:.3} ms, process page faults during it {:.0} (median a step {:.0}) (fixed step {:.3}, collision {:.3}, camera {:.3}); off-CPU time per step median {:.3} / max {:.3} ms; thread priority {}",
            player_ms.get(worst).copied().unwrap_or(f64::NAN),
            player_ran.get(worst).copied().unwrap_or(f64::NAN),
            player_faults.get(worst).copied().unwrap_or(f64::NAN),
            percentile(&player_faults, 50.0),
            step_ms.get(worst).copied().unwrap_or(f64::NAN),
            pump_ms.get(worst).copied().unwrap_or(f64::NAN),
            camera_ms.get(worst).copied().unwrap_or(f64::NAN),
            percentile(&player_off_cpu, 50.0),
            percentile(&player_off_cpu, 100.0),
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
            "player_step_on_cpu_ms": summary(&player_ran),
            "player_step_off_cpu_ms": summary(&player_off_cpu),
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
    }

    struct App {
        world: World,
        args: Args,
        gfx: Option<Gfx>,
        cw: CollisionWorld,
        player: Player,
        map: ActionMap,
        raw: RawInput,
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
        let mut cw = collision_world(&world, opts.collision_threads)?;
        let s = crate::bench::spawn(); // the spawn on the scaled plan
        let mut feet = DVec3::new(s[0] as f64, world.elevation.ground(s[0] as f64, s[1] as f64), s[1] as f64);
        cw.load_blocking(feet, pworld::LOAD_RADIUS_M);
        feet.y = cw.surface_y(feet.x, feet.z);
        let mut player = Player::new(feet, 0.0);
        let yaw = player.yaw;
        player.spawn_vehicle(&mut cw, feet.x - yaw.cos() * 4.0, feet.z + yaw.sin() * 4.0, yaw);
        let event_loop = EventLoop::new()?;
        let mut app = App {
            world,
            args: args.clone(),
            gfx: None,
            cw,
            player,
            map: ActionMap::default(),
            raw: RawInput::default(),
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
            let opts = GpuOptions { indirect_validation: self.args.indirect_validation, force_no_indirect_count: self.args.no_indirect_count, ..Default::default() };
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
            self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, agents, visual: Default::default(), audio, props, pacer });
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
            let frame = self.map.resolve(&mut self.raw, dt, false);
            self.player.look(frame.look_yaw, frame.look_pitch);
            let input = StepInput::from_frame(&frame);
            self.player.advance(&mut self.cw, &input, dt);
            let pos = self.player.position(&self.cw);
            self.cw.pump(pos);
            let view = self.player.camera(&self.cw, dt, aspect);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            self.player_ms_max = self.player_ms_max.max(ms);
            self.player_ms_sum += ms;
            let Some(g) = self.gfx.as_mut() else { return };
            let cam = to_camera(&view, aspect as f32, self.args.far);
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
            g.props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, Some(g.agents.sim_time), dt);
            g.agents.parked = g.props.parked_agents(g.agents.lib.as_deref(), cam.eye).to_vec();
            g.agents.set_player(&mut g.visual, &self.player, &self.cw, view.body_visible, dt); // agents
            g.agents.pump(&g.ctx, &g.renderer, [pos.x, pos.z, self.player.yaw], dt);
            g.audio.frame(crate::audio::FrameIn::player(&view, dt, &self.player, &self.cw, &mut g.agents, &g.renderer, &self.world.elevation)); // audio
            g.renderer.update(&g.ctx.queue, &cam);
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            let (attach, resolve) = match &g.targets.msaa {
                Some(m) => (m, Some(&tv)),
                None => (&tv, None),
            };
            g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
            eo_render::pace::submit(&g.ctx.queue, [enc.finish()]); // counted
            g.window.pre_present_notify();
            g.ctx.queue.present(surf);
            self.fps_frames += 1;
            let el = self.fps_t.elapsed().as_secs_f64();
            if el >= 1.0 {
                let (mode, speed) = match self.player.mode {
                    Mode::OnFoot => ("on foot", self.player.ground_speed),
                    Mode::Driving(i) => ("driving", self.player.vehicles[i].forward_speed(&self.cw).abs()),
                };
                g.window.set_title(&format!(
                    "Earth Online native - play - {:.0} fps - {mode} {:.1} m/s - player {:.2} ms avg / {:.2} max - F: car, V: new car, Esc: mouse",
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

        fn spawn_car_ahead(&mut self) {
            if self.player.mode != Mode::OnFoot {
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
                                if code == KeyCode::Escape {
                                    if self.looking {
                                        self.set_looking(false);
                                    } else {
                                        el.exit();
                                    }
                                    return;
                                }
                                if code == KeyCode::KeyV && !event.repeat {
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
