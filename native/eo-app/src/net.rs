//! the online client. The local simulation is replaced by the
//! server's world; the local player is predicted and reconciled
//! (eo-net `Predictor`), everyone else is interpolated, and the agent
//! renderer draws the server's cars and walkers, the other players' bodies
//! and cars, and the predicted local player.
//!
//!   eo-app --connect ADDR [--name NAME] [--spawn X,Z,YAW] [--net-sim RTT,JITTER,LOSS]
//!       window: walk and drive online (the `--play` controls); the HUD shows the
//!       connection state, ping and players near; `--connect local` starts a
//!       server in this process (127.0.0.1) and joins it.
//!   eo-app --connect ADDR|local --net-bench [--seconds 40] [--bots N] [--net-sim ...]
//!       offscreen: the scripted walk and drive of eo-server's prediction bench,
//!       measuring the network layer's main-thread time per frame and the whole frame.
//!   eo-app --connect ADDR|local --net-capture [--bots N]
//!       offscreen: two clients (two sockets, two sessions) standing on Folsom
//!       Street facing each other; each one's view at the same server tick ->
//!       bench-out/capture-net-a.png, capture-net-b.png.
//!
//! The render thread never blocks on the network (`eo_net::client`); joining
//! builds the local collision world on a thread.

use crate::agents::{AgentRuntime, PLAYER_CAR_BODY, PlayerVisual};
use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_net::client::{ClientConfig, Event, NetClient, Phase};
use eo_net::game::{GameWorld, rebuild_player};
use eo_net::link::Conditions;
use eo_net::predict::Predictor;
use eo_net::proto::SpawnHint;
use eo_net::snapshot::{CLASS_CAR, CLASS_PLAYER, CLASS_PLAYER_CAR, CLASS_WALKER, Shown, class_of};
use eo_net::wire::read_player_state;
use eo_player::world::{BuildingSet, CollisionWorld, ElevationGround, GroundProvider};
use eo_player::{CameraView, Mode, Player, StepInput};
use eo_render::camera::Camera;
use eo_render::{GpuContext, GpuOptions, QUERIES_PER_FRAME, Renderer, Targets, read_mapped, wgpu};
use eo_sim::output::{AgentBuffer, AgentInstance};
use glam::{DVec3, Vec3};
use serde_json::json;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Mode2 {
    Window,
    Bench,
    Capture,
}

struct NetOpts {
    mode: Mode2,
    connect: String,
    name: String,
    spawn: Option<[f64; 3]>,
    cond: Conditions,
    frames: usize,
    seconds: f64,
    bots: usize,
    hud: bool,
}

/// The hook main.rs calls: None unless `--connect` is on the command line.
pub fn intercept(process_start: Instant) -> Option<Result<()>> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if !raw.iter().any(|a| a == "--connect") {
        return None;
    }
    Some(parse(&raw).and_then(|(args, o)| match o.mode {
        Mode2::Window => window::run(&args, &o, process_start),
        Mode2::Bench => bench(&args, &o, process_start),
        Mode2::Capture => capture(&args, &o, process_start),
    }))
}

/// main.rs's defaults, field by field (`Args` has no Default).
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
        house_site_camera: false,
        house_dining_meter: false,
        house_glazing: true,
        beaver_house: false,
        bench: false,
        start_underwater: false,
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
        time_of_day: eo_render::look::DEFAULT_HOUR,
        look: Default::default(),
        agents: true,
        agent_density: 1,
        rocketbox: true, // people
        props: true,
        props2: Default::default(),
        perf: Default::default(),
        landmarks: true, // landmarks
        landmarks2: true,
    }
}

fn parse(raw: &[String]) -> Result<(Args, NetOpts)> {
    let mut a = base_args();
    let mut o = NetOpts { mode: Mode2::Window, connect: String::new(), name: "player".into(), spawn: None, cond: Conditions::NONE, frames: 16_000, seconds: 40.0, bots: 0, hud: true };
    let mut it = raw.iter();
    while let Some(s) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| anyhow!("{s} needs a value"));
        match s.as_str() {
            "--connect" => o.connect = val()?,
            "--net-bench" => o.mode = Mode2::Bench,
            "--net-capture" => o.mode = Mode2::Capture,
            "--name" => o.name = val()?,
            "--spawn" => {
                let v: Vec<f64> = val()?.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
                o.spawn = Some([v[0], v[1], v.get(2).copied().unwrap_or(0.0)]);
            }
            "--net-sim" => o.cond = Conditions::parse(&val()?).ok_or_else(|| anyhow!("--net-sim RTT,JITTER,LOSS"))?,
            "--frames" => o.frames = val()?.parse()?,
            "--seconds" => o.seconds = val()?.parse()?,
            "--bots" => o.bots = val()?.parse()?,
            "--no-hud" => o.hud = false,
            "--width" => a.width = val()?.parse()?,
            "--height" => a.height = val()?.parse()?,
            "--water-optics" => a.look.water_optics = true,
            "--no-water-optics" => a.look.water_optics = false,
            "--no-vsync" => a.vsync = false,
            "--out" => a.out_dir = Some(val()?.into()),
            "--cache" => a.cache_dir = Some(val()?.into()),
            "--no-cache" => a.no_cache = true,
            "--no-kit" => a.kit = false,
            "--no-models" => a.models = false,
            "--no-props" => a.props = false,
            "--no-agents" => a.agents = false,
            "--no-rocketbox" => a.rocketbox = false, // people
            "--no-thread-priority" => a.perf.thread_priority = false,
            "--no-submit-pacing" => a.perf.submit_pacing = false,
            "--time-of-day" => a.time_of_day = val()?.parse()?,
            // read by main's `groundscale_flags`.
            "--ground-scale" | "--height-scale" | "--people-scale" | "--vehicle-scale" => {
                val()?;
            }
            "--low-camera" | "--default-camera" => {}
            other => anyhow::bail!("--connect: unknown argument {other} (see native/eo-app/src/net.rs)"),
        }
    }
    if o.connect.is_empty() {
        anyhow::bail!("--connect ADDR|local");
    }
    a.bench = o.mode != Mode2::Window;
    Ok((a, o))
}

/// The collision world and its parts, as `--play` builds them (the server builds the same).
fn game_world(world: &World) -> Result<Arc<GameWorld>> {
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let buildings = Arc::new(BuildingSet::from_city(&world.city, lift));
    let elevation = Arc::new(world.elevation.clone());
    let ground: Arc<dyn GroundProvider> = match &world.street {
        Some(s) => Arc::new(eo_player::street_ground::StreetGround(s.world.clone())),
        None => Arc::new(ElevationGround(elevation.clone())),
    };
    Ok(Arc::new(GameWorld::from_parts(world.city.clone(), elevation, ground, buildings, world.street.is_some())))
}

/// A server in this process (`--connect local`), with optional bots near `around`.
struct LocalServer {
    ctl: Arc<Mutex<eo_server::predict::Ctl>>,
    thread: Option<std::thread::JoinHandle<()>>,
    bots: Option<eo_server::bots::BotRun>,
    addr: std::net::SocketAddr,
}

impl LocalServer {
    fn start(world: &World, game: &Arc<GameWorld>, bot_count: usize, around: [f64; 2]) -> Result<Self> {
        let source = Arc::new(eo_sim::native::load(&world.paths)?);
        let ctl = Arc::new(Mutex::new(eo_server::predict::Ctl::default()));
        let cfg = eo_server::ServerConfig { bind: "127.0.0.1:0".into(), player_threads: 4, sim_threads: 4, streets: world.street.is_some(), ..Default::default() };
        let thread = eo_server::predict::spawn_server(cfg, game.clone(), source, ctl.clone());
        let addr = loop {
            if let Some(a) = ctl.lock().unwrap().addr {
                break a;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let bots = if bot_count > 0 {
            let graph = Arc::new(eo_server::bots::RoadGraph::new(eo_server::bots::bot_roads(&world.city)));
            let cfg = eo_server::bots::BotConfig { server: addr, count: bot_count, threads: 1, spawn: eo_server::bots::Spawn::Around { x: around[0], z: around[1], radius: 120.0 }, drive_share: 0.5, drive_mps: 9.0, seed: 3, conditions: Conditions::NONE, send_hz: 30.0 };
            Some(eo_server::bots::BotRun::start(cfg, graph)?)
        } else {
            None
        };
        eprintln!("net: local server on {addr}{}", if bot_count > 0 { format!(" with {bot_count} bots") } else { String::new() });
        Ok(Self { ctl, thread: Some(thread), bots, addr })
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(b) = self.bots.take() {
            let _ = b.stop();
        }
        self.ctl.lock().unwrap().stop = true;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn server_addr(world: &World, game: &Arc<GameWorld>, o: &NetOpts, around: [f64; 2]) -> Result<(std::net::SocketAddr, Option<LocalServer>)> {
    if o.connect == "local" {
        let s = LocalServer::start(world, game, o.bots, around)?;
        Ok((s.addr, Some(s)))
    } else {
        Ok((o.connect.parse()?, None))
    }
}

/// Remote player paints (the civilian set's, by player id).
const PAINTS: [&str; 6] = ["#2f5d8c", "#d9d4c7", "#1d1f22", "#6b7a3a", "#c9a227", "#7c2f3e"];

/// The online client's per-frame work on the render thread.
pub struct NetSession {
    pub client: NetClient,
    pub pred: Option<Predictor>,
    joining: Option<Receiver<Result<(Player, CollisionWorld, u32)>>>,
    game: Arc<GameWorld>,
    pub visual: PlayerVisual,
    remote: HashMap<u32, (f64, f64)>,
    travelled: HashMap<u32, f64>,
    shown: Vec<Shown>,
    last_ack: u32,
    frame: eo_sim::world::Frame,
    out: Vec<(u32, eo_net::wire::NetInput)>,
    pub you: u32,
    pub players_near: usize,
    pub sim_time: f64,
    pub tick: f64,
    pub corrections: u64,
    /// This frame's main-thread network time (events, corrections, acks, inputs handed over, the view, the instances), ms.
    pub net_ms: f64,
    pub step_ms: f64,
}

impl NetSession {
    pub fn connect(game: Arc<GameWorld>, cfg: ClientConfig) -> Result<Self> {
        Ok(Self {
            client: NetClient::connect(cfg)?,
            pred: None,
            joining: None,
            game,
            visual: PlayerVisual::default(),
            remote: HashMap::new(),
            travelled: HashMap::new(),
            shown: vec![],
            last_ack: 0,
            frame: eo_sim::world::Frame { regions: vec![AgentBuffer::default()], ..Default::default() },
            out: vec![],
            you: 0,
            players_near: 0,
            sim_time: 0.0,
            tick: 0.0,
            corrections: 0,
            net_ms: 0.0,
            step_ms: 0.0,
        })
    }

    pub fn joined(&self) -> bool {
        self.pred.is_some()
    }

    /// Events (join, corrections), the server's report on our player. Cheap; never blocks.
    pub fn poll(&mut self) {
        let t0 = Instant::now();
        for ev in self.client.take_events() {
            match ev {
                Event::Accepted(a) => {
                    // Joining: the collision world around the player is built on a thread.
                    let game = self.game.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::Builder::new()
                        .name("eo-net-join".into())
                        .spawn(move || {
                            let r = (|| -> Result<(Player, CollisionWorld, u32)> {
                                let s = read_player_state(&a.state).map_err(|e| anyhow!("{e}"))?;
                                let mut cw = game.collision_world(2);
                                let p = rebuild_player(&mut cw, &s, &a.car_spawns);
                                Ok((p, cw, a.input_seq))
                            })();
                            let _ = tx.send(r);
                        })
                        .expect("spawn join thread");
                    self.joining = Some(rx);
                }
                Event::Correction(c) => {
                    if let Some(p) = self.pred.as_mut() {
                        if p.correct(&c).is_some() {
                            self.corrections += 1;
                        }
                    }
                }
                Event::Reliable(_) => {}
            }
        }
        if let Some(rx) = &self.joining {
            if let Ok(r) = rx.try_recv() {
                self.joining = None;
                match r {
                    Ok((p, cw, seq)) => match self.pred.as_mut() {
                        Some(pr) => pr.reset(p, cw, seq),
                        None => self.pred = Some(Predictor::new(p, cw, seq)),
                    },
                    Err(e) => eprintln!("net: join failed: {e:#}"),
                }
            }
        }
        if let (Some(p), Some(o)) = (self.pred.as_mut(), self.client.own()) {
            if o.ack_input != self.last_ack {
                self.last_ack = o.ack_input;
                p.on_own(&o);
            }
        }
        self.net_ms = t0.elapsed().as_secs_f64() * 1e3;
    }

    /// The local player: one drawn frame of input, predicted at once, handed to the network thread.
    pub fn step(&mut self, input: &StepInput, dt: f64) {
        let Some(p) = self.pred.as_mut() else { return };
        self.out.clear();
        let t0 = Instant::now();
        p.frame(input, dt, &mut self.out);
        self.step_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        self.client.push_inputs(&self.out, p.check(), p.applied);
        self.net_ms += t1.elapsed().as_secs_f64() * 1e3;
    }

    /// The local player's camera, with the decaying correction offset.
    pub fn camera(&mut self, dt: f64, aspect: f64) -> Option<(CameraView, DVec3)> {
        let p = self.pred.as_mut()?;
        let off = p.offset(dt);
        let mut v = p.player.camera(&p.cw, dt, aspect);
        v.eye += off;
        Some((v, off))
    }

    /// The world to draw at render tick `tick` (None: now): the server's cars
    /// and walkers to the agent feed, the other players and the local player
    /// as extra instances.
    pub fn draw(&mut self, agents: &mut AgentRuntime, tick: Option<f64>, dt: f64, body_visible: bool, offset: DVec3) {
        let t0 = Instant::now();
        let view = match tick {
            Some(t) => self.client.view_at(t),
            None => self.client.view(Instant::now()),
        };
        agents.extras.clear();
        let Some(v) = view else {
            self.net_ms += t0.elapsed().as_secs_f64() * 1e3;
            return;
        };
        self.you = v.b.header.you;
        self.tick = v.tick;
        self.sim_time = v.sim_time;
        eo_net::snapshot::interpolate(&v.a, &v.b, v.tick, 15.0, &mut self.shown);
        let me = self.pred.as_ref().map(|p| p.player.position(&p.cw)).unwrap_or(DVec3::ZERO);
        let origin = [(me.x / 256.0).floor() * 256.0, (me.z / 256.0).floor() * 256.0];
        let buf = &mut self.frame.regions[0];
        buf.origin = origin;
        buf.time = v.sim_time;
        buf.vehicles.clear();
        buf.walkers.clear();
        self.frame.time = v.sim_time;
        self.frame.steps += 1;
        buf.step = self.frame.steps;
        let lib = agents.lib.clone();
        let mine = self.you & 0x3FFF_FFFF;
        let mut players = 0;
        for s in &self.shown {
            let local = s.id & 0x3FFF_FFFF;
            let pos = [(s.pos[0] - origin[0]) as f32, 0.0, (s.pos[2] - origin[1]) as f32];
            match class_of(s.id) {
                CLASS_CAR => {
                    // Wheel spin from the travel integrated here (the wire carries 16 m of it).
                    let t = self.travelled.entry(s.id).or_insert(s.anim * 16.0);
                    *t += s.speed * dt;
                    buf.vehicles.push(AgentInstance { pos, heading: s.yaw as f32, speed: s.speed as f32, anim: (*t % 10_000.0) as f32, kind: s.kind as u16, flags: s.flags, id: local });
                }
                CLASS_WALKER => {
                    buf.walkers.push(AgentInstance { pos, heading: s.yaw as f32, speed: s.speed as f32, anim: (s.anim * std::f64::consts::TAU) as f32, kind: s.kind as u16, flags: s.flags, id: local });
                }
                CLASS_PLAYER if local != mine => {
                    players += 1;
                    let Some(lib) = &lib else { continue };
                    let r = self.remote.entry(s.id).or_insert((0.0, 0.0));
                    let height = lib.meta.family_heights.first().copied().unwrap_or(1.75) as f64;
                    r.0 = eo_agents::gait::advance_phase(r.0, s.speed, dt, height);
                    if s.flags & 1 == 0 {
                        if let Some(mut b) = eo_agents::feed::player_body(lib, s.pos, s.yaw, s.speed, r.0, v.sim_time) {
                            if !lib.meta.people_models.is_empty() {
                                b.model = lib.meta.people_models[s.kind as usize % lib.meta.people_models.len()];
                            }
                            agents.extras.push(b);
                        }
                    }
                }
                CLASS_PLAYER_CAR if local / 16 != mine => {
                    let Some(lib) = &lib else { continue };
                    let r = self.remote.entry(s.id).or_insert((0.0, 0.0));
                    r.1 += s.speed * dt / 0.33;
                    let q = eo_server::compose_ypr(s.yaw, s.pitch, s.roll);
                    if let Some(c) = eo_agents::feed::player_car(lib, PLAYER_CAR_BODY, s.pos, [q.x, q.y, q.z, q.w], r.1, s.steer, s.speed, s.flags & 1 != 0, PAINTS[s.kind as usize % PAINTS.len()]) {
                        agents.extras.push(c);
                    }
                }
                _ => {}
            }
        }
        self.players_near = players;
        if self.travelled.len() > 4 * buf.vehicles.len() + 256 {
            let live: std::collections::HashSet<u32> = self.shown.iter().map(|s| s.id).collect();
            self.travelled.retain(|k, _| live.contains(k));
            self.remote.retain(|k, _| live.contains(k));
        }
        agents.remote = Some(self.frame.clone());
        // The local player, predicted (drawn with the correction offset), in
        // the same family and paint the others see it in.
        if let (Some(p), Some(lib)) = (self.pred.as_ref(), lib.as_ref()) {
            let n0 = agents.extras.len();
            self.visual.instances(lib, &p.player, &p.cw, body_visible, dt, &mut agents.extras);
            let look = mine % 251;
            for e in &mut agents.extras[n0..] {
                e.pos[0] += offset.x as f32;
                e.pos[1] += offset.y as f32;
                e.pos[2] += offset.z as f32;
                if e.model == lib.meta.player_model && !lib.meta.people_models.is_empty() {
                    e.model = lib.meta.people_models[look as usize % lib.meta.people_models.len()];
                } else {
                    e.color = eo_agents::hex_packed(PAINTS[look as usize % PAINTS.len()]);
                }
            }
        }
        self.net_ms += t0.elapsed().as_secs_f64() * 1e3;
    }

    pub fn hud(&self) -> (String, String) {
        let st = self.client.status();
        let title = match st.phase {
            Phase::Connected if self.joined() => "Online".to_string(),
            Phase::Connected => "Joining".to_string(),
            p => p.label().to_string(),
        };
        let sub = if st.phase == Phase::Connected {
            format!("Ping {:.0} ms · {} player{} near", st.rtt_ms, self.players_near, if self.players_near == 1 { "" } else { "s" })
        } else {
            format!("{} · {}", st.phase.label(), self.client.config().server)
        };
        (title, sub)
    }
}

fn to_camera(v: &CameraView, aspect: f32, far: f32) -> Camera {
    let f = v.forward.normalize_or(DVec3::Z);
    let yaw = f.x.atan2(-f.z) as f32;
    let pitch = f.y.clamp(-1.0, 1.0).asin() as f32;
    Camera { fov_y_deg: v.fov_deg as f32, far, ..Camera::new(Vec3::new(v.eye.x as f32, v.eye.y as f32, v.eye.z as f32), yaw, pitch, aspect) }
}

fn client_config(addr: std::net::SocketAddr, name: &str, o: &NetOpts, spawn: [f64; 3], seed: u64) -> ClientConfig {
    let mut c = ClientConfig::new(addr, &format!("dev:{name}"));
    c.conditions = o.cond;
    c.seed = seed;
    c.spawn = Some(SpawnHint { x: spawn[0] as f32, z: spawn[1] as f32, yaw: spawn[2] as f32, driving: false });
    c
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({ "median": pct(v, 50.0), "p95": pct(v, 95.0), "max": pct(v, 100.0), "n": v.len() })
}

struct Offscreen {
    ctx: GpuContext,
    renderer: Renderer,
    color: wgpu::Texture,
    view: wgpu::TextureView,
    unorm: wgpu::TextureView,
    targets: Targets,
    models: ModelRuntime,
}

fn offscreen(world: &mut World, args: &Args, eye: Vec3, process_start: Instant) -> Result<Offscreen> {
    let opts = GpuOptions { indirect_validation: args.indirect_validation, force_no_indirect_count: args.no_indirect_count, ..Default::default() };
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (renderer, bases) = make_renderer(&ctx, world, args, format, false)?;
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("net-offscreen"),
        size: wgpu::Extent3d { width: args.width, height: args.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    let view = color.create_view(&Default::default());
    let unorm = color.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8Unorm), ..Default::default() });
    let targets = Targets::new(&ctx.device, args.width, args.height, format, 1);
    let models = ModelRuntime::start(world, args, bases, eye, process_start);
    Ok(Offscreen { ctx, renderer, color, view, unorm, targets, models })
}

fn save_png(ctx: &GpuContext, tex: &wgpu::Texture, path: &std::path::Path) -> Result<()> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("net capture"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
    ctx.queue.submit([enc.finish()]);
    let bytes = read_mapped(ctx, &buf)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + w as usize * 4]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(file, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn out_dir(args: &Args) -> std::path::PathBuf {
    args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"))
}

/// Pump the static world, props and agents until resident (or `timeout_s`).
fn settle(g: &mut Offscreen, world: &World, props: &mut crate::props::PropsRuntime, agents: &mut AgentRuntime, cam: &Camera, timeout_s: f64) -> Result<f64> {
    let t = Instant::now();
    g.models.set_camera(cam.eye);
    loop {
        g.models.pump(&g.ctx, &g.renderer, world, cam.eye)?;
        props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, None, 0.0);
        agents.pump(&g.ctx, &g.renderer, [cam.eye.x as f64, cam.eye.z as f64, 0.0], 0.0);
        g.renderer.update(&g.ctx.queue, cam);
        g.ctx.queue.submit([]);
        let _ = g.ctx.device.poll(wgpu::PollType::Poll);
        if (g.models.settled() && props.settled() && agents.lib.is_some()) || t.elapsed().as_secs_f64() > timeout_s {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

fn wait_joined(sessions: &mut [&mut NetSession], timeout_s: f64) -> Result<f64> {
    let t = Instant::now();
    while !sessions.iter().all(|s| s.joined()) {
        if t.elapsed().as_secs_f64() > timeout_s {
            let st: Vec<String> = sessions.iter().map(|s| format!("{:?}", s.client.status().phase)).collect();
            anyhow::bail!("not joined after {timeout_s} s: {st:?}");
        }
        for s in sessions.iter_mut() {
            s.poll();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

// ---------------------------------------------------------------- bench

fn bench(args: &Args, o: &NetOpts, process_start: Instant) -> Result<()> {
    let raised = args.perf.thread_priority && eo_render::hitch::raise_render_thread();
    eo_render::alloc::claim_render_thread();
    eo_render::hitch::set_worker_priority_lowered(args.perf.thread_priority);
    let mut world = load_world(args)?;
    let game = game_world(&world)?;
    let (route, spawn) = eo_server::predict::script_route(&game);
    let spawn = o.spawn.unwrap_or(spawn);
    let (addr, local) = server_addr(&world, &game, o, [spawn[0], spawn[1]])?;
    let mut s = NetSession::connect(game.clone(), client_config(addr, &o.name, o, spawn, 1))?;
    let join_ms = wait_joined(&mut [&mut s], 120.0)?;
    let aspect = args.width as f32 / args.height as f32;
    let first = {
        let (v, _) = s.camera(1.0 / 60.0, aspect as f64).unwrap();
        to_camera(&v, aspect, args.far)
    };
    let mut g = offscreen(&mut world, args, first.eye, process_start)?;
    let mut agents = AgentRuntime::start_remote(&world, args);
    let mut props = crate::props::PropsRuntime::start(&world, args);
    let settle_ms = settle(&mut g, &world, &mut props, &mut agents, &first, 600.0)?;
    // Let the world fill around the player before measuring.
    for _ in 0..120 {
        s.poll();
        s.step(&StepInput::default(), 1.0 / 60.0);
        s.draw(&mut agents, None, 1.0 / 60.0, true, DVec3::ZERO);
        agents.pump(&g.ctx, &g.renderer, [first.eye.x as f64, first.eye.z as f64, 0.0], 1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(16));
    }
    let ts_frames = (4096 / QUERIES_PER_FRAME) as usize;
    // `--frames` caps the run; the script runs on wall time (`seconds`, as a player would).
    let seconds = o.seconds;
    let max_frames = o.frames.max(1);
    let mut sets: Vec<wgpu::QuerySet> = vec![];
    let period = g.ctx.queue.get_timestamp_period() as f64;
    let mut script = eo_server::predict::Script::new(route, {
        let p = &s.pred.as_ref().unwrap().player;
        let v = &p.vehicles[0];
        let d = v.datum(&s.pred.as_ref().unwrap().cw);
        [d.x, d.z]
    });
    let (mut net_ms, mut step_ms, mut agents_ms, mut props_ms, mut cpu_ms) = (vec![], vec![], vec![], vec![], vec![]);
    let mut subs: Vec<wgpu::SubmissionIndex> = vec![];
    let mut last = Instant::now();
    let total_steps = (seconds * 60.0) as usize;
    let (mut walked, mut drove) = (0usize, 0usize);
    let mut players_near = 0;
    let mut entities = 0usize;
    let t_run = Instant::now();
    let mut frames = 0;
    let mut pacer = eo_render::pace::WaitPacer::new(args.perf.submit_pacing);
    for i in 0..max_frames {
        if t_run.elapsed().as_secs_f64() > seconds {
            break;
        }
        frames = i + 1;
        if i % ts_frames == 0 {
            sets.push(g.ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("net-ts"), ty: wgpu::QueryType::Timestamp, count: QUERIES_PER_FRAME * ts_frames as u32 }));
        }
        let mut charged_ms = 0.0; // a prepay the wait did not absorb
        if i >= 2 {
            let (r, w) = pacer.around(&g.ctx.device, &g.ctx.queue, || g.ctx.device.poll(wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }));
            r.map_err(|e| anyhow!("{e}"))?;
            charged_ms = w.charged_ms;
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        let t0 = Instant::now();
        s.poll();
        let input = {
            let p = s.pred.as_mut().unwrap();
            let k = p.stats.steps as usize;
            script.step_input(k, total_steps, &mut p.player, &p.cw)
        };
        s.step(&input, dt);
        match s.pred.as_ref().unwrap().player.mode {
            Mode::OnFoot => walked += 1,
            Mode::Driving(_) => drove += 1,
        }
        let (view, off) = s.camera(dt, aspect as f64).unwrap();
        s.draw(&mut agents, None, dt, view.body_visible, off);
        let n = s.net_ms;
        let cam = to_camera(&view, aspect, args.far);
        g.models.set_camera(cam.eye);
        g.models.pump(&g.ctx, &g.renderer, &world, cam.eye)?;
        let tp = Instant::now();
        props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, Some(s.sim_time), dt);
        if props.on {
            agents.parked = props.parked_agents(agents.lib.as_deref(), cam.eye).to_vec();
        }
        props_ms.push(tp.elapsed().as_secs_f64() * 1e3);
        let ta = Instant::now();
        agents.pump(&g.ctx, &g.renderer, [cam.eye.x as f64, cam.eye.z as f64, 0.0], dt);
        agents_ms.push(ta.elapsed().as_secs_f64() * 1e3);
        g.renderer.update(&g.ctx.queue, &cam);
        let mut enc = g.ctx.device.create_command_encoder(&Default::default());
        let ts = sets.get(i / ts_frames).map(|q| (q, QUERIES_PER_FRAME * (i % ts_frames) as u32));
        g.renderer.encode(&mut enc, &g.view, None, &g.targets.depth, ts);
        subs.push(eo_render::pace::submit(&g.ctx.queue, [enc.finish()])); // counted
        cpu_ms.push(t0.elapsed().as_secs_f64() * 1e3 + charged_ms);
        net_ms.push(n);
        step_ms.push(s.step_ms);
        players_near = players_near.max(s.players_near);
        entities = entities.max(s.frame.regions[0].vehicles.len() + s.frame.regions[0].walkers.len());
    }
    g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    let mut ts = vec![];
    for (k, q) in sets.iter().enumerate() {
        let n = QUERIES_PER_FRAME * (frames - k * ts_frames).min(ts_frames) as u32;
        let rb = g.ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ts-resolve"), size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let mb = g.ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ts-read"), size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = g.ctx.device.create_command_encoder(&Default::default());
        enc.resolve_query_set(q, 0..n, &rb, 0);
        enc.copy_buffer_to_buffer(&rb, 0, &mb, 0, n as u64 * 8);
        g.ctx.queue.submit([enc.finish()]);
        ts.extend(read_mapped(&g.ctx, &mb)?.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())));
    }
    let qp = eo_render::look::Q_POST as usize;
    let mut gpu_ms = vec![];
    let mut total_ms = vec![];
    for i in 0..frames {
        let b = i * QUERIES_PER_FRAME as usize;
        let gm = ts[b + qp + 1].wrapping_sub(ts[b]) as f64 * period / 1e6;
        gpu_ms.push(gm);
        total_ms.push(cpu_ms[i] + gm);
    }
    // The first frames of the path (pipelines' first draws) are warm-up, as in the other benches.
    const WARMUP: usize = 60;
    let worst: Vec<serde_json::Value> = {
        let mut idx: Vec<usize> = (0..frames).collect();
        idx.sort_by(|a, b| total_ms[*b].total_cmp(&total_ms[*a]));
        idx.iter().take(6).map(|&i| json!({ "frame": i, "total_ms": total_ms[i], "cpu_ms": cpu_ms[i], "gpu_ms": gpu_ms[i], "net_ms": net_ms[i], "step_ms": step_ms[i] })).collect()
    };
    eprintln!("worst frames: {}", serde_json::to_string(&worst)?);
    let w = WARMUP.min(frames.saturating_sub(1));
    let (net_ms, step_ms, agents_ms, props_ms, cpu_ms, gpu_ms, total_ms) = (net_ms[w..].to_vec(), step_ms[w..].to_vec(), agents_ms[w..].to_vec(), props_ms[w..].to_vec(), cpu_ms[w..].to_vec(), gpu_ms[w..].to_vec(), total_ms[w..].to_vec());
    let st = s.client.status();
    let p = s.pred.as_ref().unwrap();
    let pass = |v: &[f64], b: f64| pct(v, 50.0) <= b;
    let net_ok = pass(&net_ms, 0.3) && pct(&net_ms, 100.0) <= 1.0;
    let frame_ok = pct(&total_ms, 50.0) < 10.0 && pct(&total_ms, 95.0) < 10.0 && pct(&total_ms, 100.0) < 10.0;
    println!(
        "\n[net client, {} frames, {} conditions, {} bots] walked {walked} / drove {drove} frames; {} players and up to {entities} agents near\n  NETWORK main thread ms: median {:.4} p95 {:.4} max {:.4}  [{}] (budget: median <= 0.3, no frame > 1.0)\n  predicted player step ms: median {:.3} p95 {:.3} max {:.3}\n  agents pump {:.3} / {:.3} / {:.3}; props {:.3} / {:.3} / {:.3}\n  WHOLE FRAME (CPU through submit + GPU) ms: median {:.3} p95 {:.3} max {:.3}  [{}]   CPU {:.3} / {:.3} / {:.3}  GPU {:.3} / {:.3} / {:.3}\n  ping {:.0} ms, interpolation delay {:.0} ms, snapshots {} (lost {}), corrections {}, error after reconciliation max {:.1} mm over {} acks; join {join_ms:.0} ms, settle {settle_ms:.0} ms; render thread priority raised: {raised}",
        frames,
        if o.cond.is_none() { "clean".to_string() } else { format!("{:?}", o.cond.send) },
        o.bots,
        players_near,
        pct(&net_ms, 50.0),
        pct(&net_ms, 95.0),
        pct(&net_ms, 100.0),
        if net_ok { "PASS" } else { "FAIL" },
        pct(&step_ms, 50.0),
        pct(&step_ms, 95.0),
        pct(&step_ms, 100.0),
        pct(&agents_ms, 50.0),
        pct(&agents_ms, 95.0),
        pct(&agents_ms, 100.0),
        pct(&props_ms, 50.0),
        pct(&props_ms, 95.0),
        pct(&props_ms, 100.0),
        pct(&total_ms, 50.0),
        pct(&total_ms, 95.0),
        pct(&total_ms, 100.0),
        if frame_ok { "PASS" } else { "FAIL" },
        pct(&cpu_ms, 50.0),
        pct(&cpu_ms, 95.0),
        pct(&cpu_ms, 100.0),
        pct(&gpu_ms, 50.0),
        pct(&gpu_ms, 95.0),
        pct(&gpu_ms, 100.0),
        st.rtt_ms,
        st.interp_delay_ms,
        st.snapshots,
        st.snapshots_lost,
        p.stats.corrections,
        p.stats.error_mm.iter().copied().fold(0.0, f64::max),
        p.stats.error_mm.len(),
    );
    let dir = out_dir(args);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("net-client-bench.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "frames": frames, "bots": o.bots, "conditions": format!("{:?}", o.cond),
            "frames_walking": walked, "frames_driving": drove, "players_near_max": players_near, "agents_near_max": entities,
            "net_main_thread_ms": summary(&net_ms), "net_definition": "eo-app net.rs NetSession: events, corrections (restore + replay), acks, inputs handed to the network thread, the interpolated view, the agent frame and the remote players' instances",
            "predicted_step_ms": summary(&step_ms), "agents_pump_ms": summary(&agents_ms), "props_pump_ms": summary(&props_ms),
            "frame_cpu_ms": summary(&cpu_ms), "frame_gpu_ms": summary(&gpu_ms), "frame_total_ms": summary(&total_ms),
            "net_verdict": if net_ok { "PASS" } else { "FAIL" }, "frame_verdict": if frame_ok { "PASS" } else { "FAIL" },
            "ping_ms": st.rtt_ms, "interp_delay_ms": st.interp_delay_ms, "snapshots": st.snapshots, "snapshots_lost": st.snapshots_lost,
            "corrections": p.stats.corrections, "error_after_reconciliation_mm": summary(&p.stats.error_mm),
            "join_ms": join_ms, "settle_ms": settle_ms,
        }))?,
    )?;
    println!("wrote {}", path.display());
    drop(local);
    Ok(())
}

// ---------------------------------------------------------------- captures

/// Two players on Folsom Street's sidewalk 9 m apart, facing each other.
fn capture_spots(world: &World) -> ([f64; 3], [f64; 3]) {
    let s = crate::bench::spawn();
    let (sx, sz) = (s[0] as f64, s[1] as f64);
    // The nearest street way to the spawn: its direction and width.
    let mut best = (f64::MAX, [1.0, 0.0], [sx, sz], 2u32);
    for r in world.city.roads.iter().filter(|r| matches!(r.highway.as_str(), "primary" | "secondary" | "tertiary" | "residential")) {
        for w in r.points64.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dz * dz;
            if l2 < 1.0 {
                continue;
            }
            let t = (((sx - a[0]) * dx + (sz - a[1]) * dz) / l2).clamp(0.0, 1.0);
            let p = [a[0] + dx * t, a[1] + dz * t];
            let d = (p[0] - sx).hypot(p[1] - sz);
            if d < best.0 {
                let l = l2.sqrt();
                best = (d, [dx / l, dz / l], p, r.lanes.max(2));
            }
        }
    }
    let (_, dir, c, lanes) = best;
    let n = [-dir[1], dir[0]];
    let side = lanes as f64 * 3.05 / 2.0 + 2.6;
    let a = [c[0] + n[0] * side, c[1] + n[1] * side];
    let b = [a[0] + dir[0] * 9.0, a[1] + dir[1] * 9.0];
    let ya = (b[0] - a[0]).atan2(b[1] - a[1]);
    let yb = (a[0] - b[0]).atan2(a[1] - b[1]);
    ([a[0], a[1], ya], [b[0], b[1], yb])
}

fn capture(args: &Args, o: &NetOpts, process_start: Instant) -> Result<()> {
    let mut world = load_world(args)?;
    let game = game_world(&world)?;
    let (sa, sb) = capture_spots(&world);
    let (addr, local) = server_addr(&world, &game, o, [sa[0], sa[1]])?;
    let mut a = NetSession::connect(game.clone(), client_config(addr, "ada", o, sa, 11))?;
    let mut b = NetSession::connect(game.clone(), client_config(addr, "bo", o, sb, 22))?;
    wait_joined(&mut [&mut a, &mut b], 120.0)?;
    let aspect = args.width as f32 / args.height as f32;
    let cam_of = |s: &mut NetSession| {
        let (v, _) = s.camera(1.0 / 60.0, aspect as f64).unwrap();
        to_camera(&v, aspect, args.far)
    };
    let first = cam_of(&mut a);
    let mut g = offscreen(&mut world, args, first.eye, process_start)?;
    let mut agents = AgentRuntime::start_remote(&world, args);
    let mut props = crate::props::PropsRuntime::start(&world, args);
    settle(&mut g, &world, &mut props, &mut agents, &first, 600.0)?;
    let land = eo_data::LandCover::load(&world.paths).ok();
    let mut ui = eo_ui::Ui::new(&g.ctx.device, &g.ctx.queue, Some(&world.paths.repo_root), None, Some(eo_ui::MapInputs { city: &world.city, land: land.as_ref(), backdrop_centre: [sa[0] as f32, sa[1] as f32] }));
    ui.set_screen(eo_ui::Screen::Hud);
    // Both stand still, facing each other, while the street fills (10 s).
    let t0 = Instant::now();
    let mut last = Instant::now();
    while t0.elapsed().as_secs_f64() < 10.0 {
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        for (s, spot) in [(&mut a, sa), (&mut b, sb)] {
            s.poll();
            if let Some(p) = s.pred.as_mut() {
                p.player.yaw = spot[2];
            }
            s.step(&StepInput::default(), dt);
            s.draw(&mut agents, None, dt, true, DVec3::ZERO);
        }
        agents.pump(&g.ctx, &g.renderer, [sa[0], sa[1], 0.0], dt);
        std::thread::sleep(Duration::from_millis(8));
    }
    // One server tick for both views: a little behind both clients' render ticks.
    let tick = a.client.render_tick(Instant::now()).unwrap().min(b.client.render_tick(Instant::now()).unwrap()).floor() - 2.0;
    let dir = out_dir(args);
    std::fs::create_dir_all(&dir)?;
    let mut report = vec![];
    for (name, s) in [("a", &mut a), ("b", &mut b)] {
        let cam = cam_of(s);
        // The agents at that tick, frozen while the feed grounds them and the frames settle.
        for _ in 0..12 {
            s.poll();
            s.draw(&mut agents, Some(tick), 0.0, true, DVec3::ZERO);
            g.models.set_camera(cam.eye);
            g.models.pump(&g.ctx, &g.renderer, &world, cam.eye)?;
            props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, Some(s.sim_time), 0.0);
            if props.on {
                agents.parked = props.parked_agents(agents.lib.as_deref(), cam.eye).to_vec();
            }
            agents.pump(&g.ctx, &g.renderer, [cam.eye.x as f64, cam.eye.z as f64, 0.0], 0.0);
            g.renderer.update(&g.ctx.queue, &cam);
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            g.renderer.encode(&mut enc, &g.view, None, &g.targets.depth, None);
            g.ctx.queue.submit([enc.finish()]);
            g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
            std::thread::sleep(Duration::from_millis(15));
        }
        // The HUD: connection state and ping.
        if o.hud {
            let (title, sub) = s.hud();
            let p = s.pred.as_ref().unwrap();
            let f = p.player.character.feet();
            ui.hud.player = [f.x as f32, f.z as f32];
            ui.hud.heading = (std::f64::consts::PI - p.player.yaw) as f32;
            ui.hud.hour = args.time_of_day as f32;
            ui.hud.objective = Some((title, sub));
            ui.set_time(100.0);
            for k in 0..3 {
                ui.set_time(100.0 + k as f64 * 0.3);
                let mut enc = g.ctx.device.create_command_encoder(&Default::default());
                ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, args.width, args.height, &eo_ui::Input::default());
                g.ctx.queue.submit([enc.finish()]);
            }
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            g.renderer.update(&g.ctx.queue, &cam);
            g.renderer.encode(&mut enc, &g.view, None, &g.targets.depth, None);
            ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, args.width, args.height, &eo_ui::Input::default());
            ui.draw(&g.ctx.device, &mut enc, &g.unorm, wgpu::TextureFormat::Rgba8Unorm, None);
            g.ctx.queue.submit([enc.finish()]);
        }
        let path = dir.join(format!("capture-net-{name}.png"));
        save_png(&g.ctx, &g.color, &path)?;
        let agents_in_view = s.frame.regions[0].vehicles.len() + s.frame.regions[0].walkers.len();
        let ids: Vec<u32> = s.shown.iter().filter(|e| matches!(class_of(e.id), CLASS_CAR)).map(|e| e.id).collect();
        report.push(json!({ "client": name, "player_id": s.client.player_id(), "tick": tick, "sim_time": s.sim_time, "agents": agents_in_view, "cars": ids.len(), "players_near": s.players_near, "hud": s.hud(), "camera": [cam.eye.x, cam.eye.y, cam.eye.z] }));
    }
    // The same traffic in both: the cars both hold at that tick, and how far apart their states are.
    let va = a.client.view_at(tick).unwrap();
    let vb = b.client.view_at(tick).unwrap();
    let mut both = 0;
    let mut differ = 0;
    let mb: HashMap<u32, eo_net::snapshot::QState> = vb.b.entities.iter().copied().collect();
    for (id, s) in va.b.entities.iter() {
        if let Some(t) = mb.get(id) {
            if matches!(class_of(*id), CLASS_CAR | CLASS_WALKER | CLASS_PLAYER | CLASS_PLAYER_CAR) {
                both += 1;
                differ += (s != t) as usize;
            }
        }
    }
    println!("capture: tick {tick}: the two clients share {both} entities in that snapshot, {differ} differ (same snapshot tick: {})", va.b.header.tick == vb.b.header.tick);
    std::fs::write(dir.join("capture-net.json"), serde_json::to_vec_pretty(&json!({ "views": report, "shared_entities": both, "differing": differ, "same_snapshot_tick": va.b.header.tick == vb.b.header.tick, "spots": [sa, sb] }))?)?;
    drop(a);
    drop(b);
    drop(local);
    Ok(())
}

// ---------------------------------------------------------------- window

mod window {
    use super::*;
    use eo_player::input::{ActionMap, RawInput};
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
        agents: AgentRuntime,
        props: crate::props::PropsRuntime,
        ui: eo_ui::Ui,
        pacer: eo_render::pace::WaitPacer, // the driver's submission spike, taken in the swapchain wait
    }

    struct App {
        world: World,
        args: Args,
        gfx: Option<Gfx>,
        session: NetSession,
        _local: Option<LocalServer>,
        map: ActionMap,
        raw: RawInput,
        looking: bool,
        last: Instant,
        fps_frames: u32,
        fps_t: Instant,
        net_ms_max: f64,
        process_start: Instant,
        spawn: [f64; 3],
        clock: Instant,
    }

    pub fn run(args: &Args, o: &NetOpts, process_start: Instant) -> Result<()> {
        let world = load_world(args)?;
        let game = game_world(&world)?;
        let s = crate::bench::spawn();
        let spawn = o.spawn.unwrap_or([s[0] as f64, s[1] as f64, 0.0]);
        let (addr, local) = server_addr(&world, &game, o, [spawn[0], spawn[1]])?;
        let session = NetSession::connect(game, client_config(addr, &o.name, o, spawn, std::process::id() as u64))?;
        let event_loop = EventLoop::new()?;
        let mut app = App {
            world,
            args: args.clone(),
            gfx: None,
            session,
            _local: local,
            map: ActionMap::default(),
            raw: RawInput::default(),
            looking: false,
            last: Instant::now(),
            fps_frames: 0,
            fps_t: Instant::now(),
            net_ms_max: 0.0,
            process_start,
            spawn,
            clock: Instant::now(),
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    impl App {
        fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
            let attrs = Window::default_attributes().with_title("Earth Online native - online").with_inner_size(PhysicalSize::new(self.args.width, self.args.height));
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
            let plain = format.remove_srgb_suffix();
            if plain != format {
                config.view_formats = vec![plain];
            }
            config.present_mode = if self.args.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            surface.configure(&ctx.device, &config);
            let (renderer, base) = make_renderer(&ctx, &self.world, &self.args, format, false)?;
            let targets = Targets::new(&ctx.device, config.width, config.height, format, 1);
            let eye = Vec3::new(self.spawn[0] as f32, 0.0, self.spawn[1] as f32);
            let models = ModelRuntime::start(&mut self.world, &self.args, base, eye, self.process_start);
            let agents = AgentRuntime::start_remote(&self.world, &self.args);
            let props = crate::props::PropsRuntime::start(&self.world, &self.args);
            let land = eo_data::LandCover::load(&self.world.paths).ok();
            let mut ui = eo_ui::Ui::new(&ctx.device, &ctx.queue, Some(&self.world.paths.repo_root), None, Some(eo_ui::MapInputs { city: &self.world.city, land: land.as_ref(), backdrop_centre: [self.spawn[0] as f32, self.spawn[1] as f32] }));
            ui.set_screen(eo_ui::Screen::Hud);
            let pacer = eo_render::pace::WaitPacer::new(self.args.perf.submit_pacing);
            self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, agents, props, ui, pacer });
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
            let Some(g) = self.gfx.as_mut() else { return };
            let aspect = g.config.width as f64 / g.config.height.max(1) as f64;
            let s = &mut self.session;
            s.poll();
            let frame = self.map.resolve(&mut self.raw, dt, false);
            if let Some(p) = s.pred.as_mut() {
                p.player.look(frame.look_yaw, frame.look_pitch);
            }
            s.step(&StepInput::from_frame(&frame), dt);
            let (cam, body, off) = match s.camera(dt, aspect) {
                Some((v, off)) => (to_camera(&v, aspect as f32, self.args.far), v.body_visible, off),
                None => {
                    // Not joined yet: look down the street from the spawn.
                    let e = Vec3::new(self.spawn[0] as f32, self.world.elevation.ground(self.spawn[0], self.spawn[1]) as f32 + 1.7, self.spawn[1] as f32);
                    (Camera { far: self.args.far, ..Camera::new(e, self.spawn[2] as f32, 0.0, aspect as f32) }, true, DVec3::ZERO)
                }
            };
            s.draw(&mut g.agents, None, dt, body, off);
            self.net_ms_max = self.net_ms_max.max(s.net_ms);
            let (acquired, _) = g.pacer.around(&g.ctx.device, &g.ctx.queue, || g.surface.get_current_texture());
            let surf = match acquired {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                _ => {
                    g.surface.configure(&g.ctx.device, &g.config);
                    return;
                }
            };
            let view = surf.texture.create_view(&Default::default());
            let plain = g.config.format.remove_srgb_suffix();
            let (ui_view, ui_format) = if g.config.view_formats.contains(&plain) { (surf.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(plain), ..Default::default() }), plain) } else { (surf.texture.create_view(&Default::default()), g.config.format) };
            g.models.set_camera(cam.eye);
            if let Err(e) = g.models.pump(&g.ctx, &g.renderer, &self.world, cam.eye) {
                eprintln!("model streaming: {e:#}");
            }
            g.props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, Some(s.sim_time), dt);
            if g.props.on {
                g.agents.parked = g.props.parked_agents(g.agents.lib.as_deref(), cam.eye).to_vec();
            }
            g.agents.pump(&g.ctx, &g.renderer, [cam.eye.x as f64, cam.eye.z as f64, 0.0], dt);
            g.renderer.update(&g.ctx.queue, &cam);
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            g.renderer.encode(&mut enc, &view, None, &g.targets.depth, None);
            // HUD: the connection state and ping in the objective chip.
            let (title, sub) = s.hud();
            if let Some(p) = s.pred.as_ref() {
                let f = p.player.position(&p.cw);
                g.ui.hud.player = [f.x as f32, f.z as f32];
                g.ui.hud.heading = (std::f64::consts::PI - p.player.yaw) as f32;
                g.ui.hud.mode = match p.player.mode {
                    Mode::OnFoot => eo_ui::HudMode::OnFoot,
                    Mode::Driving(i) => eo_ui::HudMode::Drive { speed_kph: (p.player.vehicles[i].forward_speed(&p.cw).abs() * 3.6) as f32 },
                };
            }
            g.ui.hud.hour = self.args.time_of_day as f32;
            g.ui.hud.objective = Some((title, sub));
            g.ui.set_time(self.clock.elapsed().as_secs_f64());
            let _ = g.ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, g.config.width, g.config.height, &eo_ui::Input::default());
            g.ui.draw(&g.ctx.device, &mut enc, &ui_view, ui_format, None);
            eo_render::pace::submit(&g.ctx.queue, [enc.finish()]); // counted
            g.window.pre_present_notify();
            g.ctx.queue.present(surf);
            self.fps_frames += 1;
            let el = self.fps_t.elapsed().as_secs_f64();
            if el >= 1.0 {
                let st = s.client.status();
                g.window.set_title(&format!(
                    "Earth Online native - online - {:.0} fps - {} - ping {:.0} ms - net {:.3} ms max - F: car, Esc: mouse",
                    self.fps_frames as f64 / el,
                    st.phase.label(),
                    st.rtt_ms,
                    self.net_ms_max
                ));
                self.fps_frames = 0;
                self.net_ms_max = 0.0;
                self.fps_t = Instant::now();
            }
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
                            g.targets = Targets::new(&g.ctx.device, s.width, s.height, g.config.format, 1);
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
