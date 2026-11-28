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
//!   eo-app --connect local --net-shots FILE.json --out DIR [--pair-before DIR2]
//!       offscreen: landmark scenes (`net_shots`): bot players walk up to a
//!       viewpoint and hold emotes; stills from the shots' cameras, and frame
//!       sequences of one player walking up and stopping. `--pair-before`
//!       draws every still and frame a second time from the same instant
//!       with `--no-remote-bodies --no-name-tags` into DIR2: matched pairs
//!       whose street people and traffic are the same server state.
//!
//! PRESENCE (kill switches, each restoring the previous drawing):
//!   --no-remote-bodies  other players drawn as before: the first walk style
//!                       and idle loop, the linear snapshot path, no emotes
//!                       (and no selfie phone; street people may wear a
//!                       player's body again)
//!   --no-name-tags      no name tags over them (and no tag depth probe)
//!   --no-player-contact no ground contact under their bodies (the patches
//!                       that darken the ground by the sky and sun each
//!                       shoe, shin, thigh and crouched seat hides:
//!                       `eo_agents::rig::contacts`)
//!   --no-shared-sky     the hour and weather are this client's own
//!                       (`--time-of-day`, `--weather`), not the server's
//! In the window: 1 wave, 2 point, 3 sit (crouch), 4 photo, 0 stop;
//! T fast-travels to the next landmark viewpoint.
//!
//! The render thread never blocks on the network (`eo_net::client`); joining
//! builds the local collision world on a thread.

use crate::agents::{AgentRuntime, PlayerVisual};
use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_agents::remote::{RemoteBody, RemoteInput, emote_family};
use eo_net::client::{ClientConfig, Event, NetClient, Phase};
use eo_net::game::{GameWorld, rebuild_player};
use eo_net::presence::{Emote, SkyClock, SkySmoother, Viewpoint, flag, unpack_anim};
use eo_net::proto::{TravelReply, TravelResult, msg, read_roster};
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
    Shots,
}

struct NetOpts {
    vehicle: eo_player::vehicle::VehicleConfig,
    mode: Mode2,
    connect: String,
    name: String,
    spawn: Option<[f64; 3]>,
    cond: Conditions,
    frames: usize,
    seconds: f64,
    bots: usize,
    hud: bool,
    presence: Presence,
    /// `--net-shots FILE.json`: the landmark scenes (`--shots-only a,b`: the scenes whose name contains a or b).
    shots: Option<std::path::PathBuf>,
    only: Option<String>,
    /// `--pair-before DIR`: every capture again from the same instant with the previous drawing.
    pair_before: Option<std::path::PathBuf>,
    /// The local server's sky (`--time-of-day`, `--weather`, `--day-rate`).
    weather: u8,
    weather_seed: u32,
    weather_start_tick: u32,
    weather_physics: bool,
    day_rate: f64,
}

/// What the client draws of the other players (each a kill switch).
#[derive(Clone, Copy, Debug)]
pub struct Presence {
    /// Rocketbox bodies moved by `eo_agents::remote` (off: the previous drawing).
    pub bodies: bool,
    pub tags: bool,
    /// The server's hour and weather (off: this client's own).
    pub shared_sky: bool,
    /// Ground contact under the bodies (`eo_agents::rig::contact_instances`).
    pub contact: bool,
}

impl Default for Presence {
    fn default() -> Self {
        Self { bodies: true, tags: true, shared_sky: true, contact: true }
    }
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
        Mode2::Shots => net_shots::run(&args, &o, process_start),
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
        time_of_day: eo_render::look::DEFAULT_HOUR,
        look: Default::default(),
        agents: true,
        agent_density: 1,
        rocketbox: true, // people
        test_character: None,
        props: true,
        props2: Default::default(),
        perf: Default::default(),
        landmarks: true, // landmarks
        landmarks2: true,
        structures: None, // `--structures` (quality::Switches)
        facade_clutter: 0, // `--facade-clutter` (quality::Switches)
        player_contact: false, // below, from `Presence`
        landmark_access: true,
        car_detail: 0, // `--car-detail` (quality::Switches)
    }
}

fn parse(raw: &[String]) -> Result<(Args, NetOpts)> {
    let mut a = base_args();
    let mut o = NetOpts { vehicle: eo_player::vehicle::VehicleConfig { model: eo_player::vehicle::VehicleModel::Sedan, ..Default::default() }, mode: Mode2::Window, connect: String::new(), name: "player".into(), spawn: None, cond: Conditions::NONE, frames: 16_000, seconds: 40.0, bots: 0, hud: true, presence: Presence::default(), shots: None, only: None, pair_before: None, weather: 0, weather_seed: a.look.sky.seed, weather_start_tick: 0, weather_physics: true, day_rate: 1.0 };
    let mut tier = crate::quality::Switches::default(); // `--quality` and the lighting switches
    let mut it = raw.iter();
    while let Some(s) = it.next() {
        if let Some(n) = tier.take(s, it.as_slice().first().map(String::as_str))? {
            it.by_ref().take(n).for_each(drop); // the value it took, if any
            continue;
        }
        let mut val = || it.next().cloned().ok_or_else(|| anyhow!("{s} needs a value"));
        match s.as_str() {
            "--connect" => o.connect = val()?,
            "--net-bench" => o.mode = Mode2::Bench,
            "--net-capture" => o.mode = Mode2::Capture,
            "--vehicle" => o.vehicle.model = eo_player::vehicle::VehicleModel::parse(&val()?)?,
            "--cargo-kg" => o.vehicle.cargo_kg = val()?.parse()?,
            "--occupants-kg" => o.vehicle.occupants_kg = val()?.parse()?,
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
            "--no-remote-bodies" => o.presence.bodies = false,
            "--no-name-tags" => o.presence.tags = false,
            "--no-player-contact" => o.presence.contact = false,
            "--traffic-contact" | "--no-traffic-contact" => {} // read when starting the embedded server
            "--no-shared-sky" => o.presence.shared_sky = false,
            "--net-shots" => {
                o.mode = Mode2::Shots;
                o.shots = Some(val()?.into());
            }
            "--day-rate" => o.day_rate = val()?.parse()?,
            "--weather-seed" => {
                a.look.sky.seed = val()?.parse()?;
                o.weather_seed = a.look.sky.seed;
            }
            "--world-time" => {
                a.look.sky.world_time_s = val()?.parse()?;
                o.weather_start_tick = eo_net::weather::start_tick(a.look.sky.world_time_s)?;
            }
            "--weather" => {
                let w = val()?;
                o.weather = eo_net::presence::weather_code(&w).ok_or_else(|| anyhow!("--weather {w}: expected a weather preset or schedule"))?;
                a.look.sky.weather = match eo_sky::Kind::parse(&w) {
                    Some(k) => eo_render::sky::WeatherMode::Preset(k),
                    None => eo_render::sky::WeatherMode::Schedule,
                };
            }
            "--settle-timeout" => {
                val()?;
            }
            "--shots-only" => o.only = Some(val()?),
            "--pair-before" => o.pair_before = Some(val()?.into()),
            "--width" => a.width = val()?.parse()?,
            "--height" => a.height = val()?.parse()?,
            "--water-optics" => a.look.water_optics = true,
            "--underwater-habitats" => a.underwater_habitats = true,
            "--no-underwater-habitats" => a.underwater_habitats = false,
            "--no-water-optics" => a.look.water_optics = false,
            "--volumetric-clouds" => a.look.sky.volumetric_clouds = true,
            "--no-volumetric-clouds" => a.look.sky.volumetric_clouds = false,
            "--weather-fx" => a.look.sky.weather_fx = true,
            "--no-weather-fx" => a.look.sky.weather_fx = false,
            "--dive-exposure" => a.look.dive_exposure = true,
            "--no-dive-exposure" => a.look.dive_exposure = false,
            "--bridge-detail" | "--no-bridge-detail" => {} // read by `crate::bridge_detail`
            "--no-vsync" => a.vsync = false,
            "--out" => a.out_dir = Some(val()?.into()),
            "--cache" => a.cache_dir = Some(val()?.into()),
            "--no-cache" => a.no_cache = true,
            "--no-kit" => a.kit = false,
            "--no-models" => a.models = false,
            "--no-props" => a.props = false,
            // the furniture walk's poster hoardings (eo-props `furniture::set_poster_hoardings`).
            "--poster-hoardings" | "--no-poster-hoardings" => {
                a.props2.parse_flag(s);
            }
            "--no-landmark-access" => a.landmark_access = false, // the landmark decks and lifts (a local server follows)
            "--no-agents" => a.agents = false,
            "--no-rocketbox" => a.rocketbox = false, // people
            "--no-thread-priority" => a.perf.thread_priority = false,
            "--no-submit-pacing" => a.perf.submit_pacing = false,
            "--time-of-day" => a.time_of_day = val()?.parse()?,
            "--no-photo-mode" | "--no-dof" => {} // read by photo.rs
            // read by main's `groundscale_flags`.
            "--ground-scale" | "--height-scale" | "--people-scale" | "--vehicle-scale" => {
                val()?;
            }
            "--low-camera" | "--default-camera" => {}
            s if eo_landmarks::suspension::SWITCHES.contains(&s) => {} // eo_landmarks::suspension::features
            "--no-external" => {} // main's `external_flag`
            "--traffic-street-plan" | "--no-traffic-street-plan" | "--no-traffic-passing" | "--no-traffic-model-footprints"
            | "--traffic-bend-braking" | "--no-traffic-bend-braking" => {}
            other => anyhow::bail!("--connect: unknown argument {other} (see native/eo-app/src/net.rs)"),
        }
    }
    tier.apply(&mut a);
    // The contact patches' shading compiled in only when they are drawn (`ShaderFeatures::player_contact`).
    a.player_contact = o.presence.bodies && o.presence.contact;
    if o.connect.is_empty() {
        anyhow::bail!("--connect ADDR|local");
    }
    a.bench = o.mode != Mode2::Window;
    o.vehicle.validated()?;
    o.weather_physics = a.look.sky.weather_fx;
    Ok((a, o))
}

/// The collision world and its parts, as `--play` builds them (the server builds the same).
fn game_world(world: &World, access: bool) -> Result<Arc<GameWorld>> {
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let elevation = Arc::new(world.elevation.clone());
    let ground: Arc<dyn GroundProvider> = match &world.street {
        Some(s) => Arc::new(eo_player::street_ground::StreetGround::new(s.world.clone(), world.structures.clone())),
        None => Arc::new(ElevationGround(elevation.clone())),
    };
    let decks = eo_net::game::city_decks(&world.paths, &elevation, ground.as_ref(), access)?;
    let mut buildings = BuildingSet::from_city(&world.city, lift);
    buildings.open_tunnel_approaches(world.structures.as_deref());
    let buildings = Arc::new(buildings.with_decks(decks));
    // `with_decks`: the in-process server keeps the built decks too, so its world matches a dedicated server's (`--structures`).
    Ok(Arc::new(GameWorld::from_parts(world.city.clone(), elevation, ground, buildings, world.street.is_some(), crate::structures::options(world)).with_decks(world.structures.clone())))
}

/// A server in this process (`--connect local`), with optional bots near `around`.
struct LocalServer {
    ctl: Arc<Mutex<eo_server::predict::Ctl>>,
    thread: Option<std::thread::JoinHandle<()>>,
    bots: Option<eo_server::bots::BotRun>,
    addr: std::net::SocketAddr,
}

impl NetOpts {
    fn server_sky(&self, hour: f64, day_rate: f64, weather: u8) -> eo_server::ServerConfig {
        eo_server::ServerConfig { hour, day_rate, weather, weather_seed: self.weather_seed,
            traffic_contact: crate::last_switch("traffic-contact").unwrap_or(false),
            weather_start_tick: self.weather_start_tick, weather_physics: self.weather_physics, ..Default::default() }
    }
}

impl LocalServer {
    fn start(world: &World, game: &Arc<GameWorld>, bot_count: usize, around: [f64; 2], sky: eo_server::ServerConfig) -> Result<Self> {
        // The structure level of each way (`--structures`; empty otherwise, which leaves the sim as it was).
        let levels = world.structures.as_ref().map(|s| s.way_levels().clone()).unwrap_or_default();
        let mut source = eo_sim::native::load(&world.paths)?.with_levels(levels);
        if crate::agents::traffic_street_plan_on() {
            source = source.with_traffic_plan(game.ground.traffic_plan())
                .with_passing(crate::agents::traffic_passing_on());
            if let Some(decks) = &game.decks {
                source = source.with_deck_surfaces(decks.traffic_surfaces());
            }
        }
        let source = Arc::new(eo_server::configure_traffic_previews(source)?);
        let ctl = Arc::new(Mutex::new(eo_server::predict::Ctl::default()));
        let cfg = eo_server::ServerConfig { bind: "127.0.0.1:0".into(), player_threads: 4, sim_threads: 4, streets: world.street.is_some(), structures: game.structures, ..sky };
        let thread = eo_server::predict::spawn_server(cfg, game.clone(), source, ctl.clone());
        let addr = loop {
            if let Some(a) = ctl.lock().unwrap().addr {
                break a;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let bots = if bot_count > 0 {
            let graph = Arc::new(eo_server::bots::RoadGraph::new(eo_server::bots::bot_roads(&world.city)));
            let cfg = eo_server::bots::BotConfig { server: addr, count: bot_count, threads: 1, spawn: eo_server::bots::Spawn::Around { x: around[0], z: around[1], radius: 120.0 }, drive_share: 0.5, drive_mps: 9.0, seed: 3, conditions: Conditions::NONE, send_hz: 30.0, structures: game.structures, visits: vec![] };
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

/// The local server's starting hour: `--time-of-day` as given (main.rs's default otherwise).
fn world_hour(o: &NetOpts) -> f64 {
    let _ = o;
    let raw: Vec<String> = std::env::args().collect();
    raw.iter().position(|a| a == "--time-of-day").and_then(|i| raw.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(eo_render::look::DEFAULT_HOUR)
}

fn server_addr(world: &World, game: &Arc<GameWorld>, o: &NetOpts, around: [f64; 2]) -> Result<(std::net::SocketAddr, Option<LocalServer>)> {
    if o.connect == "local" {
        let s = LocalServer::start(world, game, o.bots, around, o.server_sky(world_hour(o), o.day_rate, o.weather))?;
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
    joining: Option<Receiver<Result<(Player, CollisionWorld, u32, eo_net::weather::WeatherEpoch)>>>,
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
    /// What is drawn of the other players.
    pub presence: Presence,
    /// Player names from the server's roster.
    pub names: HashMap<u32, String>,
    bodies: HashMap<u32, RemoteBody>,
    /// The other players drawn this frame (feet, facing, emote), for the name tags.
    pub others: Vec<Other>,
    tag_alpha: HashMap<u32, f32>,
    /// The name tags' depth tests for the next GPU probe, and each head's
    /// visible share from the last probe read back (0 hidden .. 1 clear).
    pub probe_asks: Vec<ProbeAsk>,
    pub head_visible: HashMap<u32, f32>,
    /// The faces on screen this frame (the other players' and the street
    /// people's heads) the tags' plates keep off (`eo_ui::Ui::tag_avoid`).
    pub tag_avoid: Vec<[f32; 4]>,
    sky: SkySmoother,
    /// The server's sky as drawn this frame, and as last handed to the renderer.
    pub sky_shown: Option<SkyClock>,
    sky_applied: Option<SkyClock>,
    /// Where this player's cars were spawned (a world rebuilt for an arrival spawns them there first).
    cars: Vec<[f32; 3]>,
    /// A fast trip under way: the server's reply, and the world being loaded round the arrival.
    travel: Option<(TravelReply, Option<Receiver<CollisionWorld>>)>,
    prepared: Option<CollisionWorld>,
    pub travel_note: Option<(String, Instant)>,
    pub viewpoints: Vec<Viewpoint>,
    next_viewpoint: usize,
    /// Draw this client's own body and cars (off for a capture's camera player).
    pub draw_self: bool,
    /// Draw the other players' cars (off in the landmark captures, where every bot's parked car would stand in the scene).
    pub player_cars: bool,
}

/// One other player as drawn this frame.
#[derive(Clone, Debug)]
pub struct Other {
    pub id: u32,
    pub feet: [f64; 3],
    pub emote: Emote,
    pub speed: f64,
    /// Where its head is in the drawn pose (`eo_agents::rig`; None without a body).
    pub head: Option<eo_agents::rig::Head>,
}

/// One name tag's depth test, asked of the GPU (`TagProbe`): nine points
/// over the head (window pixels) and the depth a nearer occluder would
/// exceed (reversed Z: the head centre moved `TAG_OCCLUDER_M` toward the eye).
#[derive(Clone, Debug)]
pub struct ProbeAsk {
    pub id: u32,
    pub px: [[f32; 2]; 9],
    pub depth: f32,
}

/// Seconds a name tag takes to fade in or out (occlusion, distance).
const TAG_FADE_S: f64 = 0.15;
/// The distance fade: opaque to 12 m, 0.35 at 25 m, gone at 40 m (a face is
/// recognisable to about 20 to 25 m in daylight, the "social distance" of
/// public-space studies; past that a label is clutter, and past 40 m it only
/// covers the scene).
const TAG_NEAR_M: f64 = 12.0;
const TAG_MID_M: f64 = 25.0;
const TAG_MID_ALPHA: f64 = 0.35;
const TAG_FAR_M: f64 = 40.0;
/// The tag hangs this far over the head joint (pose-aware: a crouch lowers it).
const TAG_OVER_HEAD_M: f64 = 0.25;
/// Without a body to read the head from: this far over the feet (a 1.75 m body plus a hand's breadth).
const TAG_OVER_FEET_M: f64 = 2.02;
const TAG_OVER_FEET_CROUCHED_M: f64 = 1.45;
/// The second line (what the player is doing) is shown nearer than this.
const TAG_DETAIL_M: f64 = 12.0;
/// The name's size at 1600 x 900: 18 px at 7 m, falling as the square root
/// of distance; a plate under 14 px tall is not drawn. The second line is
/// at least 12 px.
const TAG_NAME_PX: f64 = 18.0;
const TAG_NAME_AT_M: f64 = 7.0;
const TAG_MIN_PLATE_PX: f64 = 14.0;
const TAG_DETAIL_MIN_PX: f64 = 12.0;
/// Something this much nearer the eye than the head's centre hides it (a
/// head is about 0.2 m deep: its own face, hair or a hand at it do not).
const TAG_OCCLUDER_M: f64 = 0.30;
/// The visible share of the head under which a tag is hidden, and over which it is clear; between, it is at 0.3.
const TAG_HIDDEN_SHARE: f32 = 0.12;
const TAG_CLEAR_SHARE: f32 = 0.85;
const TAG_PARTLY_ALPHA: f32 = 0.3;
/// A sky clock change smaller than this is not handed to the renderer (each
/// change refits every shadow cascade): 10 s of game time.
const SKY_STEP_H: f64 = 10.0 / 3600.0;

impl NetSession {
    pub fn connect(game: Arc<GameWorld>, mut cfg: ClientConfig) -> Result<Self> {
        cfg.structures = game.structures; // the handshake claims this world's own grade-separated roads
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
            presence: Presence::default(),
            names: HashMap::new(),
            bodies: HashMap::new(),
            others: vec![],
            tag_alpha: HashMap::new(),
            probe_asks: vec![],
            head_visible: HashMap::new(),
            tag_avoid: vec![],
            sky: SkySmoother::default(),
            sky_shown: None,
            sky_applied: None,
            cars: vec![],
            travel: None,
            prepared: None,
            travel_note: None,
            viewpoints: eo_net::presence::viewpoints().unwrap_or_default(),
            next_viewpoint: 0,
            draw_self: true,
            player_cars: true,
        })
    }

    /// Start an emote (Emote::None ends one); the server shows it to everyone.
    pub fn emote(&self, e: Emote) {
        self.client.command(vec![msg::EMOTE, e as u8]);
    }

    /// Ask the server for a fast trip to viewpoint `i` (it may refuse).
    pub fn travel(&mut self, i: usize) {
        let i = i as u16;
        self.client.command(vec![msg::TRAVEL, i as u8, (i >> 8) as u8]);
    }

    /// The next landmark viewpoint, in turn (the window's T key).
    pub fn travel_next(&mut self) {
        if self.viewpoints.is_empty() {
            return;
        }
        let i = self.next_viewpoint % self.viewpoints.len();
        self.next_viewpoint += 1;
        self.travel(i);
    }

    fn on_reliable(&mut self, m: &[u8]) {
        if let Some((joined, id, name)) = read_roster(m) {
            if joined {
                self.names.insert(id, name);
            } else {
                self.names.remove(&id);
            }
            return;
        }
        if let Some(r) = TravelReply::read(m) {
            let name = self.viewpoints.get(r.viewpoint as usize).map(|v| v.name.clone()).unwrap_or_else(|| format!("viewpoint {}", r.viewpoint));
            if r.result != TravelResult::Accepted {
                self.travel_note = Some((format!("{name}: refused ({:?})", r.result), Instant::now()));
                return;
            }
            self.travel_note = Some((format!("Travelling to {name}"), Instant::now()));
            // The ground there is loaded off the render thread; the arrival swaps it in.
            let game = self.game.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let at = r.arrival;
            std::thread::Builder::new()
                .name("eo-net-travel".into())
                .spawn(move || {
                    let (x, z) = (at[0] as f64, at[1] as f64);
                    let mut cw = game.collision_world(2);
                    cw.load_blocking(DVec3::new(x, game.ground.ground_y(x, z), z), eo_player::world::LOAD_RADIUS_M);
                    let _ = tx.send(cw);
                })
                .expect("spawn travel thread");
            self.travel = Some((r, Some(rx)));
            self.prepared = None;
        }
    }

    /// A correction: an arrival by fast travel is adopted in the world loaded
    /// round it (when it is ready), anything else corrected in place.
    fn on_correction(&mut self, c: &eo_net::snapshot::Correction) -> Option<f64> {
        let p = self.pred.as_mut()?;
        let arrival = self.travel.as_ref().map(|t| t.0.arrival);
        let state = read_player_state(&c.state).ok();
        if let (Some(at), Some(st)) = (arrival, state.as_ref()) {
            let near = (st.character.position[0] - at[0] as f64).hypot(st.character.position[2] - at[1] as f64) < 3.0;
            if near {
                self.travel = None;
                if let Some(mut cw) = self.prepared.take() {
                    let pl = rebuild_player(&mut cw, st, &self.cars);
                    return p.adopt(pl, cw, c);
                }
            }
        }
        p.correct(c)
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
                    self.cars = a.car_spawns.clone();
                    // Joining: the collision world around the player is built on a thread.
                    let game = self.game.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::Builder::new()
                        .name("eo-net-join".into())
                        .spawn(move || {
                            let r = (|| -> Result<(Player, CollisionWorld, u32, eo_net::weather::WeatherEpoch)> {
                                let s = read_player_state(&a.state).map_err(|e| anyhow!("{e}"))?;
                                let mut cw = game.collision_world(2);
                                let p = rebuild_player(&mut cw, &s, &a.car_spawns);
                                Ok((p, cw, a.input_seq, a.weather))
                            })();
                            let _ = tx.send(r);
                        })
                        .expect("spawn join thread");
                    self.joining = Some(rx);
                }
                Event::Correction(c) => {
                    if self.on_correction(&c).is_some() {
                        self.corrections += 1;
                    }
                }
                Event::Reliable(m) => self.on_reliable(&m),
            }
        }
        if let Some((_, rx)) = self.travel.as_mut() {
            if let Some(cw) = rx.as_ref().and_then(|r| r.try_recv().ok()) {
                self.prepared = Some(cw);
                *rx = None;
            }
        }
        if let Some(rx) = &self.joining {
            if let Ok(r) = rx.try_recv() {
                self.joining = None;
                match r {
                    Ok((p, cw, seq, weather)) => match self.pred.as_mut() {
                        Some(pr) => pr.reset_with_weather(p, cw, seq, weather),
                        None => self.pred = Some(Predictor::with_weather(p, cw, seq, weather)),
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
        let ahead = (v.tick - v.b.header.tick as f64) / eo_net::proto::TICK_HZ as f64;
        self.sky_shown = Some(self.sky.step(v.b.header.sky, ahead, dt));
        if self.presence.bodies {
            eo_net::snapshot::interpolate_smooth(v.pa.as_deref(), &v.a, &v.b, v.pb.as_deref(), v.tick, 15.0, &mut self.shown);
        } else {
            eo_net::snapshot::interpolate(&v.a, &v.b, v.tick, 15.0, &mut self.shown);
        }
        self.others.clear();
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
        // The bodies the other players are drawn in (no street person wears one).
        let mut player_models: Vec<u32> = vec![];
        agents.remote_traffic.clear();
        for s in &self.shown {
            let local = s.id & 0x3FFF_FFFF;
            let pos = [(s.pos[0] - origin[0]) as f32, 0.0, (s.pos[2] - origin[1]) as f32];
            match class_of(s.id) {
                CLASS_CAR => {
                    // Wheel spin from the travel integrated here (the wire carries 16 m of it).
                    let t = self.travelled.entry(s.id).or_insert(s.anim * 16.0);
                    *t += s.speed * dt;
                    let pos = [pos[0], s.pos[1] as f32, pos[2]];
                    let physical = eo_net::snapshot::TRAFFIC_PHYSICAL;
                    if s.flags & physical != 0 {
                        let rotation = eo_server::compose_ypr(s.yaw, s.pitch, s.roll);
                        agents.remote_traffic.insert(u64::from(local), (s.pos.map(|p| p as f32), rotation.as_quat().to_array(), s.speed as f32));
                    }
                    buf.vehicles.push(AgentInstance { pos, heading: s.yaw as f32, speed: s.speed as f32, anim: (*t % 10_000.0) as f32, kind: s.kind as u16, flags: s.flags & !physical, id: local });
                }
                CLASS_WALKER => {
                    buf.walkers.push(AgentInstance { pos, heading: s.yaw as f32, speed: s.speed as f32, anim: (s.anim * std::f64::consts::TAU) as f32, kind: s.kind as u16, flags: s.flags, id: local });
                }
                CLASS_PLAYER if local != mine => {
                    players += 1;
                    let Some(lib) = &lib else { continue };
                    let (emote, count) = unpack_anim((s.anim * 256.0).round() as u8);
                    if s.flags & flag::DRIVING == 0 {
                        self.others.push(Other { id: local, feet: s.pos, emote, speed: s.speed, head: None });
                    }
                    if self.presence.bodies {
                        // Locomotion from the replicated speed and state, emotes, stops on a planted foot.
                        if s.flags & flag::DRIVING == 0 {
                            let body = self.bodies.entry(s.id).or_default();
                            let inp = RemoteInput { feet: s.pos, yaw: s.yaw, speed: s.speed, sprint: s.flags & flag::SPRINT != 0, airborne: s.flags & flag::AIRBORNE != 0, emote: emote_family(emote as u8), emote_count: count, key: local, look: s.kind as usize, time: v.sim_time, dt };
                            if let Some(b) = body.step(lib, &inp) {
                                // The head the name tag hangs over, in this pose; the selfie's phone in the hand.
                                if let Some(o) = self.others.last_mut().filter(|o| o.id == local) {
                                    o.head = eo_agents::rig::head(lib, &b);
                                }
                                if emote == Emote::Selfie {
                                    if let Some(ph) = eo_agents::rig::phone(lib, &b) {
                                        agents.extras.push(ph);
                                    }
                                }
                                if !player_models.contains(&b.model) {
                                    player_models.push(b.model);
                                }
                                // The ground under the shoes, shins and a crouch's seat.
                                if self.presence.contact {
                                    eo_agents::rig::contact_instances(lib, &b, b.pos[1], &mut agents.extras);
                                }
                                agents.extras.push(b);
                            }
                        } else {
                            self.bodies.remove(&s.id);
                        }
                        continue;
                    }
                    // `--no-remote-bodies`: the previous drawing.
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
                CLASS_PLAYER_CAR if local / 16 != mine && self.player_cars => {
                    let Some(lib) = &lib else { continue };
                    let r = self.remote.entry(s.id).or_insert((0.0, 0.0));
                    r.1 += s.speed * dt / 0.33;
                    let q = eo_server::compose_ypr(s.yaw, s.pitch, s.roll);
                    let model = eo_player::vehicle::VehicleModel::from_code(((s.flags >> 2) & 3) as u8).unwrap_or_default();
                    if let Some(c) = eo_agents::feed::player_car(lib, model.body(), s.pos, [q.x, q.y, q.z, q.w], r.1, s.steer, s.speed, s.flags & 1 != 0, PAINTS[s.kind as usize % PAINTS.len()]) {
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
            self.bodies.retain(|k, _| live.contains(k));
        }
        agents.remote = Some(self.frame.clone());
        agents.remap_people.clear();
        if let (true, Some(lib)) = (self.presence.bodies, lib.as_ref()) {
            // Each player body to the next one in the pool, of the same
            // proportion group's sex, that no player here wears.
            let pool = &lib.meta.people_models;
            let female = |m: u32| pool.iter().position(|&x| x == m).and_then(|i| lib.meta.motion.person_group.get(i)).and_then(|&g| lib.meta.motion.group_female.get(g as usize)).copied();
            for &m in &player_models {
                let Some(i) = pool.iter().position(|&x| x == m) else { continue };
                if let Some(to) = (1..pool.len()).map(|k| pool[(i + k) % pool.len()]).find(|&c| !player_models.contains(&c) && female(c) == female(m)) {
                    agents.remap_people.push((m, to));
                }
            }
        }
        // The local player, predicted (drawn with the correction offset), in
        // the same family and paint the others see it in.
        if let (Some(p), Some(lib), true) = (self.pred.as_ref(), lib.as_ref(), self.draw_self) {
            let n0 = agents.extras.len();
            self.visual.instances(lib, &p.player, &p.cw, body_visible, dt, &mut agents.extras);
            // The look the server gives this player (its account's), as the others see it.
            let look = self.shown.iter().find(|e| class_of(e.id) == CLASS_PLAYER && e.id & 0x3FFF_FFFF == mine).map(|e| e.kind as u32).unwrap_or(mine % 251);
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
        agents.mark_bores(); // the players' bodies and cars in a tunnel's bore (`--tunnels`)
        self.net_ms += t0.elapsed().as_secs_f64() * 1e3;
    }

    /// The other players as the previous drawing draws them
    /// (`--no-remote-bodies`) at the same instant, for a matched capture:
    /// `draw` with the bodies off, everything else it touches put back (the
    /// drawn sky, the players listed for the tags, the cars' wheel travel).
    pub fn previous_extras(&mut self, agents: &mut AgentRuntime, tick: Option<f64>, dt: f64, body_visible: bool) -> Vec<eo_agents::GpuAgent> {
        let keep = (self.presence, self.others.clone(), self.sky, self.sky_shown, self.players_near, self.travelled.clone(), agents.remote.clone(), self.net_ms, agents.remap_people.clone());
        self.presence.bodies = false;
        self.presence.tags = false;
        self.draw(agents, tick, dt, body_visible, DVec3::ZERO);
        let out = agents.extras.clone();
        (self.presence, self.others, self.sky, self.sky_shown, self.players_near, self.travelled, agents.remote, self.net_ms, agents.remap_people) = keep;
        out
    }

    /// The server's hour and weather to the renderer (`--no-shared-sky`:
    /// nothing; the client keeps its own). Handed over only when the drawn
    /// clock has moved 10 s of game time or the weather changed (each change
    /// refits every shadow cascade).
    pub fn apply_sky(&mut self, renderer: &Renderer, base: eo_render::sky::SkyOptions) -> Option<SkyClock> {
        if !self.presence.shared_sky {
            return None;
        }
        let sky = self.sky_shown?;
        let changed = match self.sky_applied {
            None => true,
            Some(a) => a.weather != sky.weather || a.seed != sky.seed || eo_net::presence::hour_delta(a.hour, sky.hour).abs() >= SKY_STEP_H,
        };
        if changed {
            renderer.look.set_time_of_day(sky.hour);
            let weather = match eo_sky::Kind::parse(eo_net::presence::weather_name(sky.weather)) {
                Some(k) if sky.weather != eo_net::presence::WEATHER_SCHEDULE => eo_render::sky::WeatherMode::Preset(k),
                _ => eo_render::sky::WeatherMode::Schedule,
            };
            if base != eo_render::sky::SkyOptions::off() {
                renderer.look.set_sky_options(eo_render::sky::SkyOptions { weather, seed: if sky.weather == eo_net::presence::WEATHER_SCHEDULE { sky.seed } else { base.seed }, ..base });
            }
            self.sky_applied = Some(sky);
        }
        if sky.weather == eo_net::presence::WEATHER_SCHEDULE && let Some(p) = &self.pred {
            renderer.look.sky.set_clock(p.weather.epoch.seconds(p.seq), renderer.look.sky.frame_dt());
        }
        Some(sky)
    }

    /// The name tags for this frame. Each hangs 0.25 m over its player's
    /// head joint as drawn (a crouch lowers it); fades with distance (opaque
    /// to 12 m, 0.35 at 25 m, gone at 40 m) and shrinks with it (no plate
    /// under 14 px at 1600 x 900); hides behind buildings, the ground and the
    /// players' cars (a ray against the collision world) and, from the GPU's
    /// depth probe of the last frame (`TagProbe`), behind anything nearer
    /// than the head: a passer-by, a pole, another player (partly hidden: 0.3,
    /// wholly: gone); easing in and out over 0.15 s (`dt` 0: at once). The
    /// HUD lays them out (a tag that would cover a nearer one moves up with
    /// a leader line). Also fills `probe_asks` for the next probe.
    pub fn name_tags(&mut self, cam: &Camera, width: u32, height: u32, dt: f64) -> Vec<eo_ui::NameTag> {
        self.probe_asks.clear();
        self.tag_avoid.clear();
        if !self.presence.tags {
            return vec![];
        }
        let vp = cam.view_proj();
        let eye = cam.eye.as_dvec3();
        let px = height as f64 / 900.0;
        let scale = eo_data::groundscale::people();
        let (right, up) = {
            let f = cam.forward();
            let r = cam.right();
            (r, r.cross(f).normalize())
        };
        let project = |p: Vec3| -> Option<(f32, f32, f32)> {
            let c = vp * p.extend(1.0);
            (c.w > 0.05).then(|| ((c.x / c.w * 0.5 + 0.5) * width as f32, (1.0 - (c.y / c.w * 0.5 + 0.5)) * height as f32, c.z / c.w))
        };
        // A head's box on screen: 0.11 m either side of its centre, 0.13 m over and under.
        let head_box = |c: Vec3, s: f32| -> Option<[f32; 4]> {
            let pts = [c + right * 0.11 * s, c - right * 0.11 * s, c + up * 0.13 * s, c - up * 0.13 * s];
            let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in pts {
                let (x, y, _) = project(p)?;
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            Some(b)
        };
        // The street people's faces within 35 m (their heads 1.62 m over the drawn feet).
        let buf = &self.frame.regions[0];
        for w in &buf.walkers {
            let (x, z) = (buf.origin[0] + w.pos[0] as f64, buf.origin[1] + w.pos[2] as f64);
            if (x - eye.x).hypot(z - eye.z) > 35.0 {
                continue;
            }
            let c = Vec3::new(x as f32, (self.game.ground.ground_y(x, z) + 1.62 * scale) as f32, z as f32);
            if let Some(b) = head_box(c, scale as f32) {
                self.tag_avoid.push(b);
            }
        }
        let mut out = vec![];
        let mut seen = std::collections::HashSet::new();
        for o in &self.others {
            seen.insert(o.id);
            let (anchor, centre) = match o.head {
                Some(h) => (DVec3::new(h.joint.x as f64, h.joint.y as f64 + TAG_OVER_HEAD_M * scale, h.joint.z as f64), h.centre.as_dvec3()),
                None => {
                    // Down on the heels the head is lower (the crouch loop's pelvis sits 0.3 m under standing).
                    let over = if o.emote == Emote::Sit { TAG_OVER_FEET_CROUCHED_M } else { TAG_OVER_FEET_M };
                    let a = DVec3::new(o.feet[0], o.feet[1] + over * scale, o.feet[2]);
                    (a, a - DVec3::Y * 0.3 * scale)
                }
            };
            let d = centre.distance(eye);
            let own_head = head_box(centre.as_vec3(), scale as f32).unwrap_or([0.0; 4]);
            if d < 35.0 {
                self.tag_avoid.push(own_head);
            }
            let fade = if d <= TAG_NEAR_M {
                1.0
            } else if d <= TAG_MID_M {
                1.0 - (1.0 - TAG_MID_ALPHA) * (d - TAG_NEAR_M) / (TAG_MID_M - TAG_NEAR_M)
            } else {
                (TAG_MID_ALPHA * (1.0 - (d - TAG_MID_M) / (TAG_FAR_M - TAG_MID_M))).max(0.0)
            };
            let fs = TAG_NAME_PX * (TAG_NAME_AT_M / d.max(1.0)).sqrt().min(1.0) * px;
            let big_enough = fs * 1.76 >= TAG_MIN_PLATE_PX * px;
            let a_px = project(anchor.as_vec3());
            let on_screen = a_px.is_some_and(|(x, y, _)| x > -0.05 * width as f32 && x < 1.05 * width as f32 && y > -0.05 * height as f32 && y < 1.1 * height as f32);
            let ray_hidden = match self.pred.as_ref() {
                Some(p) if d > 0.5 => p.cw.raycast(eye, (centre - eye) / d, d - 0.35, eo_player::world::layer::WORLD | eo_player::world::layer::STREET | eo_player::world::layer::VEHICLE, None).is_some(),
                _ => false,
            };
            // The depth probe's verdict on the head (none yet: clear).
            let share = self.head_visible.get(&o.id).copied().unwrap_or(1.0);
            let occl = if share < TAG_HIDDEN_SHARE {
                0.0
            } else if share < TAG_CLEAR_SHARE {
                TAG_PARTLY_ALPHA
            } else {
                1.0
            };
            let target = if on_screen && big_enough && !ray_hidden && fade > 0.0 { fade as f32 * occl } else { 0.0 };
            let a = self.tag_alpha.entry(o.id).or_insert(target);
            *a = if dt <= 0.0 { target } else { *a + (target - *a) * (1.0 - (-dt / TAG_FADE_S).exp()) as f32 };
            // The probe for the next frame: nine points over the head, and the depth of a nearer occluder.
            if on_screen && fade > 0.0 {
                let c = centre.as_vec3();
                let toward = (eye.as_vec3() - c).normalize_or_zero();
                let hs = scale as f32;
                let mut pts = [[0.0f32; 2]; 9];
                let mut ok = true;
                for (k, pt) in pts.iter_mut().enumerate() {
                    let (i, j) = ((k % 3) as f32 - 1.0, (k / 3) as f32);
                    let w = c + right * (i * 0.065 * hs) + up * ((-0.06 + j * 0.07) * hs);
                    match project(w) {
                        Some((x, y, _)) => *pt = [x, y],
                        None => ok = false,
                    }
                }
                if let (true, Some((_, _, z))) = (ok, project(c + toward * TAG_OCCLUDER_M as f32 * hs)) {
                    self.probe_asks.push(ProbeAsk { id: o.id, px: pts, depth: z });
                }
            }
            let Some((sx, sy, _)) = a_px else { continue };
            if *a < 0.01 {
                continue;
            }
            let name = self.names.get(&o.id).cloned().unwrap_or_else(|| format!("player {}", o.id));
            let detail = match o.emote {
                Emote::Wave => Some("waving"),
                Emote::Point => Some("pointing"),
                Emote::Sit => Some("crouching"),
                Emote::Selfie => Some("taking a selfie"),
                Emote::None => None,
            };
            // What they are doing only up close (past 12 m it is clutter: the pose says it).
            let detail = if d < TAG_DETAIL_M { detail.map(String::from) } else { None };
            let ds = (fs * 0.8).max(TAG_DETAIL_MIN_PX * px);
            out.push(eo_ui::NameTag { x: sx, y: sy, depth: d as f32, name, detail, alpha: *a, size: fs as f32, detail_size: ds as f32, head: own_head });
        }
        self.tag_alpha.retain(|k, _| seen.contains(k));
        self.head_visible.retain(|k, _| seen.contains(k));
        out
    }

    pub fn hud(&self) -> (String, String) {
        if let Some((note, at)) = &self.travel_note {
            if at.elapsed().as_secs_f64() < 4.0 {
                return ("Fast travel".to_string(), note.clone());
            }
        }
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

/// The name tags' depth probe: each tag's nine points over its head read
/// from the frame's depth target by a compute pass, each counted visible
/// unless something there is nearer than the head's centre by
/// `TAG_OCCLUDER_M` (reversed Z: a larger depth is nearer). The window and
/// the bench read the counts back a frame or two later (a ring of three
/// buffers, never waited on); a capture waits for them (`run_now`).
pub struct TagProbe {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    slots: Vec<ProbeSlot>,
    next: usize,
}

struct ProbeSlot {
    asks: wgpu::Buffer,
    hits: wgpu::Buffer,
    read: wgpu::Buffer,
    ids: Vec<u32>,
    /// 0 free, 1 encoded, 2 mapping, 3 mapped.
    state: Arc<std::sync::atomic::AtomicU8>,
}

const PROBE_TAGS: usize = 64;
const PROBE_POINTS: usize = PROBE_TAGS * 9;

const PROBE_WGSL: &str = r#"
@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<storage, read> asks: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> hits: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&asks)) {
        return;
    }
    let a = asks[i];
    if (a.w < 0.5) {
        hits[i] = 0u;
        return;
    }
    let dims = vec2<i32>(textureDimensions(depth));
    let p = clamp(vec2<i32>(floor(a.xy)), vec2<i32>(0), dims - vec2<i32>(1));
    let z = textureLoad(depth, p, 0);
    // Reversed Z: visible unless something there is nearer than the threshold.
    hits[i] = select(0u, 1u, z <= a.z);
}
"#;

impl TagProbe {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tag-probe"), source: wgpu::ShaderSource::Wgsl(PROBE_WGSL.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tag-probe"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("tag-probe"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("tag-probe"), layout: Some(&pl), module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let slot = || ProbeSlot {
            asks: device.create_buffer(&wgpu::BufferDescriptor { label: Some("tag-probe asks"), size: (PROBE_POINTS * 16) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            hits: device.create_buffer(&wgpu::BufferDescriptor { label: Some("tag-probe hits"), size: (PROBE_POINTS * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            read: device.create_buffer(&wgpu::BufferDescriptor { label: Some("tag-probe read"), size: (PROBE_POINTS * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            ids: vec![],
            state: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        };
        Self { pipeline, layout, slots: (0..3).map(|_| slot()).collect(), next: 0 }
    }

    /// Encode this frame's probe (after the scene, into `enc`) into a free
    /// slot; returns the slot, or None (nothing asked, or every slot busy).
    pub fn encode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, asks: &[ProbeAsk]) -> Option<usize> {
        use std::sync::atomic::Ordering;
        if asks.is_empty() {
            return None;
        }
        let k = (0..self.slots.len()).map(|i| (self.next + i) % self.slots.len()).find(|&i| self.slots[i].state.load(Ordering::Acquire) == 0)?;
        self.next = (k + 1) % self.slots.len();
        let n = asks.len().min(PROBE_TAGS);
        let mut data = vec![0f32; n * 9 * 4];
        for (t, a) in asks.iter().take(n).enumerate() {
            for (j, p) in a.px.iter().enumerate() {
                let o = (t * 9 + j) * 4;
                data[o..o + 4].copy_from_slice(&[p[0], p[1], a.depth, 1.0]);
            }
        }
        let slot = &mut self.slots[k];
        queue.write_buffer(&slot.asks, 0, bytemuck::cast_slice(&data));
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tag-probe"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &slot.asks, offset: 0, size: std::num::NonZeroU64::new((n * 9 * 16) as u64) }) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &slot.hits, offset: 0, size: std::num::NonZeroU64::new((n * 9 * 4) as u64) }) },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tag-probe"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(((n * 9) as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&slot.hits, 0, &slot.read, 0, (n * 9 * 4) as u64);
        slot.ids = asks.iter().take(n).map(|a| a.id).collect();
        slot.state.store(1, Ordering::Release);
        Some(k)
    }

    /// After the submit that carried `encode`'s pass: ask for the slot's counts.
    pub fn submitted(&mut self, k: Option<usize>) {
        use std::sync::atomic::Ordering;
        let Some(k) = k else { return };
        let slot = &self.slots[k];
        slot.state.store(2, Ordering::Release);
        let st = slot.state.clone();
        let n = slot.ids.len();
        slot.read.slice(0..(n * 9 * 4) as u64).map_async(wgpu::MapMode::Read, move |r| {
            st.store(if r.is_ok() { 3 } else { 0 }, Ordering::Release);
        });
    }

    /// Every slot whose counts arrived, oldest first, into `out` (id -> visible share of the head).
    pub fn collect(&mut self, out: &mut HashMap<u32, f32>) {
        use std::sync::atomic::Ordering;
        for i in 0..self.slots.len() {
            let k = (self.next + i) % self.slots.len();
            let slot = &mut self.slots[k];
            if slot.state.load(Ordering::Acquire) != 3 {
                continue;
            }
            let n = slot.ids.len();
            if let Ok(view) = slot.read.slice(0..(n * 9 * 4) as u64).get_mapped_range() {
                let hits: &[u32] = bytemuck::cast_slice(&view);
                for (t, id) in slot.ids.iter().enumerate() {
                    let seen = hits[t * 9..t * 9 + 9].iter().filter(|&&h| h != 0).count();
                    out.insert(*id, seen as f32 / 9.0);
                }
            }
            slot.read.unmap();
            slot.state.store(0, Ordering::Release);
        }
    }

    /// A capture: probe `depth` now and wait for the counts.
    pub fn run_now(&mut self, ctx: &GpuContext, depth: &wgpu::TextureView, asks: &[ProbeAsk], out: &mut HashMap<u32, f32>) -> Result<()> {
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        let k = self.encode(&ctx.device, &ctx.queue, &mut enc, depth, asks);
        ctx.queue.submit([enc.finish()]);
        self.submitted(k);
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
        self.collect(out);
        Ok(())
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
    c.vehicle = o.vehicle;
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
    let opts = GpuOptions { indirect_validation: args.indirect_validation, force_no_indirect_count: args.no_indirect_count, force_no_count_emulation: args.no_count_emulation, bounded_draws: args.bounded_draws, ..Default::default() };
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
    let game = game_world(&world, args.landmark_access)?;
    let (route, spawn) = eo_server::predict::script_route(&game);
    let spawn = o.spawn.unwrap_or(spawn);
    let (addr, local) = server_addr(&world, &game, o, [spawn[0], spawn[1]])?;
    let mut s = NetSession::connect(game.clone(), client_config(addr, &o.name, o, spawn, 1))?;
    s.presence = o.presence;
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
    let mut probe = TagProbe::new(&g.ctx.device);
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
    let mut tags_n = 0usize;
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
        let cam = to_camera(&view, aspect, args.far);
        // The presence work on the render thread: the sky handed over, the name tags (projection, occlusion rays).
        let tp0 = Instant::now();
        s.apply_sky(&g.renderer, args.look.sky);
        probe.collect(&mut s.head_visible);
        let tags = s.name_tags(&cam, args.width, args.height, dt);
        tags_n = tags_n.max(tags.len());
        s.net_ms += tp0.elapsed().as_secs_f64() * 1e3;
        let n = s.net_ms;
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
        // The tags' depth probe (its encode counted with the network's presence work).
        let tq = Instant::now();
        let probed = if s.presence.tags { probe.encode(&g.ctx.device, &g.ctx.queue, &mut enc, &g.targets.depth, &s.probe_asks) } else { None };
        let n = n + tq.elapsed().as_secs_f64() * 1e3;
        subs.push(eo_render::pace::submit(&g.ctx.queue, [enc.finish()])); // counted
        probe.submitted(probed);
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
            "weather_at_last_input": p.player.weather, "weather_world_seconds": p.weather.epoch.seconds(p.seq),
            "presence": { "remote_bodies": o.presence.bodies, "name_tags": o.presence.tags, "shared_sky": o.presence.shared_sky, "tags_max": tags_n },
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
    let game = game_world(&world, args.landmark_access)?;
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
    let mut ui = eo_ui::Ui::new(&g.ctx.device, &g.ctx.queue, Some(&world.paths.repo_root), None, Some(eo_ui::MapInputs { city: &world.city, tunnels: crate::structures::options(&world).is_some_and(|o| o.tunnels), land: land.as_ref(), backdrop_centre: [sa[0] as f32, sa[1] as f32] }));
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
        probe: TagProbe,
        fade: crate::fade::Fade, // the landmark lift's fade
    }

    struct App {
        world: World,
        args: Args,
        gfx: Option<Gfx>,
        /// photo mode (P), None with `--no-photo-mode`; the server's world runs on under it.
        photo: Option<crate::photo::PhotoMode>,
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
        let game = game_world(&world, args.landmark_access)?;
        let s = crate::bench::spawn();
        let spawn = o.spawn.unwrap_or([s[0] as f64, s[1] as f64, 0.0]);
        let (addr, local) = server_addr(&world, &game, o, [spawn[0], spawn[1]])?;
        let mut session = NetSession::connect(game, client_config(addr, &o.name, o, spawn, std::process::id() as u64))?;
        session.presence = o.presence;
        let event_loop = EventLoop::new()?;
        let mut app = App {
            world,
            args: args.clone(),
            gfx: None,
            photo: crate::photo::enabled().then(|| crate::photo::PhotoMode::new(true, args.time_of_day)),
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
            let opts = GpuOptions { indirect_validation: self.args.indirect_validation, force_no_indirect_count: self.args.no_indirect_count, force_no_count_emulation: self.args.no_count_emulation, bounded_draws: self.args.bounded_draws, ..Default::default() };
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
            let mut ui = eo_ui::Ui::new(&ctx.device, &ctx.queue, Some(&self.world.paths.repo_root), None, Some(eo_ui::MapInputs { city: &self.world.city, tunnels: crate::structures::options(&self.world).is_some_and(|o| o.tunnels), land: land.as_ref(), backdrop_centre: [self.spawn[0] as f32, self.spawn[1] as f32] }));
            ui.set_screen(eo_ui::Screen::Hud);
            let pacer = eo_render::pace::WaitPacer::new(self.args.perf.submit_pacing);
            let probe = TagProbe::new(&ctx.device);
            let fade = crate::fade::Fade::new(&ctx.device, format);
            self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, agents, props, ui, pacer, probe, fade });
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
            let photo_on = self.photo.as_ref().is_some_and(|p| p.on);
            if photo_on {
                // photo mode freezes the local player's input only: empty inputs go on to the server.
                s.step(&StepInput::default(), dt);
            } else {
                let frame = self.map.resolve(&mut self.raw, dt, false);
                if let Some(p) = s.pred.as_mut() {
                    p.player.look(frame.look_yaw, frame.look_pitch);
                }
                s.step(&StepInput::from_frame(&frame), dt);
            }
            let (mut cam, mut body, off) = match s.camera(dt, aspect) {
                Some((v, off)) => (to_camera(&v, aspect as f32, self.args.far), v.body_visible, off),
                None => {
                    // Not joined yet: look down the street from the spawn.
                    let e = Vec3::new(self.spawn[0] as f32, self.world.elevation.ground(self.spawn[0], self.spawn[1]) as f32 + 1.7, self.spawn[1] as f32);
                    (Camera { far: self.args.far, ..Camera::new(e, self.spawn[2] as f32, 0.0, aspect as f32) }, true, DVec3::ZERO)
                }
            };
            if let (Some(ph), true) = (self.photo.as_mut(), photo_on) {
                let mouse = [std::mem::take(&mut self.raw.mouse_dx), std::mem::take(&mut self.raw.mouse_dy)];
                let anchor = s.pred.as_ref().map(|p| crate::photo::anchor_of(p.player.position(&p.cw))).unwrap_or(ph.anchor);
                let cw = s.pred.as_ref().map(|p| &p.cw);
                ph.step(&crate::photo::Feed { held: &self.raw.held, mouse, dt, aspect, anchor, cw });
                if let Some(h) = ph.hour_to_set() {
                    g.renderer.look.set_time_of_day(h); // local only
                }
                g.renderer.photo_mode.set(Some(ph.params()));
                cam = ph.camera(aspect, self.args.far);
                body = true;
            }
            s.draw(&mut g.agents, None, dt, body, off);
            let sky = s.apply_sky(&g.renderer, self.args.look.sky);
            // The tags' depth probes that have come back (never waited on).
            let _ = g.ctx.device.poll(wgpu::PollType::Poll);
            g.probe.collect(&mut s.head_visible);
            g.ui.tags = s.name_tags(&cam, g.config.width, g.config.height, dt);
            g.ui.tag_avoid.clone_from(&s.tag_avoid);
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
            // The heads under the name tags, tested against this frame's depth.
            let probed = if s.presence.tags { g.probe.encode(&g.ctx.device, &g.ctx.queue, &mut enc, &g.targets.depth, &s.probe_asks) } else { None };
            // a landmark lift ride: the predicted player's fade (the server runs the same ride);
            // photo mode's free camera is not faded, and its frozen input cannot ride (no prompt).
            g.fade.draw(&mut enc, &view, if photo_on { 0.0 } else { s.pred.as_ref().map(|p| p.player.lift_fade()).unwrap_or(0.0) });
            g.ui.hud.prompt = if photo_on { None } else { s.pred.as_ref().and_then(|p| p.player.lift_prompt(&p.cw)).map(|p| ("E".to_string(), crate::fade::prompt_text(&p))) };
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
            // The server's shared hour; else the drawn hour follows the clock with `--smooth-time`.
            g.ui.hud.hour = sky.map(|k| k.hour).unwrap_or_else(|| if self.args.look.lighting.smooth_time { g.renderer.look.time_of_day() } else { self.args.time_of_day }) as f32;
            g.ui.hud.objective = Some((title, sub));
            // photo mode's panel in place of the HUD.
            g.ui.photo = self.photo.as_ref().filter(|p| p.on).map(|p| p.panel(aspect));
            g.ui.set_time(self.clock.elapsed().as_secs_f64());
            let _ = g.ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, g.config.width, g.config.height, &eo_ui::Input::default());
            g.ui.draw(&g.ctx.device, &mut enc, &ui_view, ui_format, None);
            eo_render::pace::submit(&g.ctx.queue, [enc.finish()]); // counted
            g.probe.submitted(probed);
            g.window.pre_present_notify();
            g.ctx.queue.present(surf);
            if let Some(ph) = self.photo.as_mut() {
                if let Some(scale) = ph.request.take() {
                    match crate::photo::capture(&g.ctx, &g.renderer, &cam, (g.config.width, g.config.height), g.config.format, scale, ph.describe(&cam)) {
                        Ok(p) => ph.say(format!("Saved {}", p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())),
                        Err(e) => ph.say(format!("Not saved: {e:#}")),
                    }
                }
            }
            self.fps_frames += 1;
            let el = self.fps_t.elapsed().as_secs_f64();
            if el >= 1.0 {
                let st = s.client.status();
                let range = s.pred.as_ref().and_then(|p| match p.player.mode {
                    Mode::Driving(i) => Some(if p.player.vehicles[i].reverse_selected() { " (R)" } else { " (D)" }),
                    Mode::OnFoot => None,
                }).unwrap_or("");
                g.window.set_title(&format!(
                    "Earth Online native - online - {:.0} fps - {}{range} - ping {:.0} ms - net {:.3} ms max - F: car, P: photo mode, Esc: mouse",
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

    impl App {
        /// P: enter or leave photo mode (the HUD comes back on leaving).
        fn toggle_photo(&mut self) {
            let (Some(ph), Some(g)) = (self.photo.as_mut(), self.gfx.as_ref()) else { return };
            if ph.on {
                if let Some(h) = ph.leave() {
                    g.renderer.look.set_time_of_day(h); // the game's hour again
                }
                g.renderer.photo_mode.set(None);
                return;
            }
            let aspect = g.config.width as f64 / g.config.height.max(1) as f64;
            let Some(p) = self.session.pred.as_mut() else { return }; // not joined yet
            let view = p.player.camera(&p.cw, 0.0, aspect);
            let cam = to_camera(&view, aspect as f32, self.args.far);
            ph.game_hour = g.renderer.look.time_of_day(); // (the clock's with `--smooth-time`)
            ph.enter(&cam, crate::photo::anchor_of(p.player.position(&p.cw)));
            self.raw.direction_pressed = false;
            self.raw.held.clear();
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
                    self.raw.direction_pressed = false;
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
                                let photo_on = self.photo.as_ref().is_some_and(|p| p.on);
                                if (code == crate::photo::TOGGLE && !event.repeat && self.photo.is_some()) || (code == KeyCode::Escape && photo_on) {
                                    self.toggle_photo();
                                    return;
                                }
                                if photo_on && self.photo.as_mut().is_some_and(|p| p.key(code)) {
                                    return;
                                }
                                if code == KeyCode::Escape {
                                    if self.looking {
                                        self.set_looking(false);
                                    } else {
                                        el.exit();
                                    }
                                    return;
                                }
                                // Presence: emotes and fast travel (sent once per press).
                                if !event.repeat {
                                    match code {
                                        KeyCode::Digit1 => self.session.emote(Emote::Wave),
                                        KeyCode::Digit2 => self.session.emote(Emote::Point),
                                        KeyCode::Digit3 => self.session.emote(Emote::Sit),
                                        KeyCode::Digit4 => self.session.emote(Emote::Selfie),
                                        KeyCode::Digit0 => self.session.emote(Emote::None),
                                        KeyCode::KeyT => self.session.travel_next(),
                                        _ => {}
                                    }
                                }
                                if code == KeyCode::KeyR {
                                    if photo_on { return; }
                                    self.raw.direction_pressed |= !event.repeat;
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

// ---------------------------------------------------------------- landmark captures

/// `eo-app --connect local --net-shots FILE.json --out DIR`: landmark
/// scenes for review. One server in this process holds every scene's bot
/// players (real protocol clients, `eo_server::bots::Visit`); each scene has
/// its own camera player (a client like any other, its own body and car not
/// drawn), standing behind the camera so the server's interest grid centres
/// there. A scene's bots wait at their start until its camera has streamed
/// in, then walk to their spots, turn and hold their emotes; the stills are
/// taken from the frozen instant after the last one settled (the other
/// players, traffic and people held there while the temporal resolve
/// converges); the name tags are drawn over the converged frame, as the HUD
/// draws them. A sequence then lets one more player walk up to the camera
/// and stop, written every 1/fps s of wall time with the world running (as
/// the game draws it). The scene file:
///
/// ```json
/// { "scenes": [ { "name": "coit", "viewpoint": "coit-tower", "time_of_day": 16, "weather": "clear",
///     "players": [ { "name": "ada", "at": [0, 12], "from": [3, 4], "face": "camera", "emote": "wave", "start_s": 0.5 } ],
///     "camera": [0, 1.6, 2],
///     "shots": [ { "name": "coit_plaza_group", "eye": [0, 1.6, 2], "look": [0, 1.3, 12], "fov": 50 } ],
///     "sequence": { "name": "coit_walkup", "player": { "name": "mo", ... }, "fps": 10, "seconds": 7, "shot": { ... } } } ] }
/// ```
///
/// Positions are [right, forward] metres in the viewpoint's frame (forward
/// is its heading); camera points add a height over the ground. "face" is
/// "camera" (toward the scene's camera), "view" (the viewpoint's heading) or
/// degrees off the heading. Every switch of the window applies, so the same
/// file renders with `--no-remote-bodies --no-name-tags` for the previous drawing.
mod net_shots {
    use super::*;
    use eo_server::bots::{BotConfig, BotRun, RoadGraph, Visit};
    use anyhow::Context;
    use serde_json::Value;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Frames drawn under the still camera before the PNG (the temporal resolve keeps 0.9 a frame).
    const CONVERGE_FRAMES: usize = 56;
    /// And for each instant of a sequence (0.9^16: a fifth of the instant before is left, under the moving body only).
    const SEQUENCE_FRAMES: usize = 16;
    /// Walking pace the scenes are timed with (eo-player's walk tier, m/s).
    const WALK_MPS: f64 = 1.6;
    /// The water's clock for every still (the shots tool's).
    const WATER_CLOCK_S: f64 = 20.0;

    #[derive(Clone, Copy)]
    struct Frame {
        o: [f64; 2],
        yaw: f64,
    }

    impl Frame {
        /// [right, forward] in the viewpoint's frame to game (x, z).
        fn at(&self, rf: [f64; 2]) -> [f64; 2] {
            let (s, c) = self.yaw.sin_cos();
            // forward (sin, cos); right of a body facing it (-cos, sin).
            [self.o[0] + rf[1] * s - rf[0] * c, self.o[1] + rf[1] * c + rf[0] * s]
        }
    }

    fn v2(v: &Value) -> Option<[f64; 2]> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
    }

    fn v3(v: &Value) -> Option<[f64; 3]> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    }

    struct Shot {
        name: String,
        eye: DVec3,
        at: DVec3,
        fov: f64,
    }

    fn shot(v: &Value, fr: &Frame, game: &GameWorld, default_name: Option<&str>) -> Result<Shot> {
        let name = v["name"].as_str().or(default_name).ok_or_else(|| anyhow!("a shot has no name"))?.to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            anyhow::bail!("shot name {name:?}: letters, digits, '-', '_' and '.'");
        }
        let p = |k: &str| -> Result<DVec3> {
            let e = v3(&v[k]).ok_or_else(|| anyhow!("shot {name}: \"{k}\" is not [right, up, forward]"))?;
            let xz = fr.at([e[0], e[2]]);
            Ok(DVec3::new(xz[0], game.ground.ground_y(xz[0], xz[1]) + e[1], xz[1]))
        };
        Ok(Shot { eye: p("eye")?, at: p("look")?, fov: v["fov"].as_f64().unwrap_or(50.0), name })
    }

    fn camera(s: &Shot, aspect: f32, far: f32) -> Camera {
        let d = s.at - s.eye;
        let yaw = d.x.atan2(-d.z) as f32;
        let pitch = d.y.atan2(d.x.hypot(d.z)) as f32;
        Camera { fov_y_deg: s.fov as f32, far, ..Camera::new(s.eye.as_vec3(), yaw, pitch, aspect) }
    }

    /// A player of a scene as a bot visit.
    fn visit(v: &Value, fr: &Frame, cam: [f64; 2], gate: &Arc<AtomicBool>, k: usize) -> Result<(Visit, f64)> {
        let name = v["name"].as_str().map(String::from).unwrap_or_else(|| format!("friend{k}"));
        let at = fr.at(v2(&v["at"]).ok_or_else(|| anyhow!("player {name}: no \"at\""))?);
        let from = v2(&v["from"]).map(|f| fr.at(f)).unwrap_or(at);
        let face = match &v["face"] {
            Value::String(s) if s == "camera" => (cam[0] - at[0]).atan2(cam[1] - at[1]),
            Value::Number(n) => fr.yaw + n.as_f64().unwrap_or(0.0).to_radians(),
            _ => fr.yaw,
        };
        let emote = match v["emote"].as_str() {
            Some(e) => Emote::parse(e).ok_or_else(|| anyhow!("player {name}: emote {e:?} is not wave, point, sit or selfie"))?,
            None => Emote::None,
        };
        let start_s = v["start_s"].as_f64().unwrap_or(0.0);
        let dist = (at[0] - from[0]).hypot(at[1] - from[1]);
        // The last metre is walked along the facing, so the stop faces the right way.
        let pre = [at[0] - face.sin() * 0.8, at[1] - face.cos() * 0.8];
        let path = if dist > 1.5 { vec![pre, at] } else { vec![at] };
        let spawn_yaw = if dist > 0.2 { (at[0] - from[0]).atan2(at[1] - from[1]) } else { face };
        // A passer-by is caught mid-walk by the stills: it does not hold them back.
        let done = if v["passerby"].as_bool().unwrap_or(false) { 0.0 } else { start_s + dist / WALK_MPS + 2.5 };
        Ok((Visit { name, spawn: [from[0], from[1], spawn_yaw], path, face, emote, start_s, emote_after_s: 0.9, sprint: v["sprint"].as_bool().unwrap_or(false), tour: None, gate: Some(gate.clone()) }, done))
    }

    struct Scene {
        name: String,
        hour: f64,
        weather: u8,
        weather_name: String,
        frame: Frame,
        camera_player: [f64; 3],
        shots: Vec<Shot>,
        gate: Arc<AtomicBool>,
        ready_s: f64,
        players: Vec<String>,
        sequence: Option<(String, Arc<AtomicBool>, f64, f64, Shot)>,
        input: Value,
    }

    pub fn run(args: &Args, o: &NetOpts, process_start: Instant) -> Result<()> {
        let path = o.shots.clone().unwrap();
        let file: Value = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("read {}", path.display()))?)?;
        let mut world = load_world(args)?;
        let game = game_world(&world, args.landmark_access)?;
        let gf = eo_data::geo::GeoFrame::of_city(&world.city.meta);
        let vps = eo_net::presence::viewpoints()?;
        let mut scenes = vec![];
        let mut visits = vec![];
        for sv in file["scenes"].as_array().ok_or_else(|| anyhow!("{}: no \"scenes\"", path.display()))? {
            let name = sv["name"].as_str().unwrap_or("scene").to_string();
            if o.only.as_ref().is_some_and(|only| !only.split(',').map(str::trim).any(|k| !k.is_empty() && name.contains(k))) {
                continue;
            }
            let vid = sv["viewpoint"].as_str().ok_or_else(|| anyhow!("scene {name}: no viewpoint"))?;
            let vp = vps.iter().find(|v| v.id == vid).ok_or_else(|| anyhow!("scene {name}: no viewpoint {vid}"))?;
            let g = vp.game(&gf);
            let frame = Frame { o: [g[0], g[1]], yaw: g[2] };
            let cam_rf = v3(&sv["camera"]).unwrap_or([0.0, 1.6, 0.0]);
            let cam = frame.at([cam_rf[0], cam_rf[2]]);
            // The camera's player stands 6 m behind the camera (out of every shot).
            let back = frame.at([cam_rf[0], cam_rf[2] - 6.0]);
            let gate = Arc::new(AtomicBool::new(false));
            let mut ready_s: f64 = 0.0;
            let mut players = vec![];
            for (k, pv) in sv["players"].as_array().into_iter().flatten().enumerate() {
                let (v, done) = visit(pv, &frame, cam, &gate, k)?;
                ready_s = ready_s.max(done);
                players.push(v.name.clone());
                visits.push(v);
            }
            let shots = sv["shots"].as_array().into_iter().flatten().map(|v| shot(v, &frame, &game, None)).collect::<Result<Vec<_>>>()?;
            let sequence = match sv.get("sequence") {
                Some(q) if !q.is_null() => {
                    let g2 = Arc::new(AtomicBool::new(false));
                    let (v, done) = visit(&q["player"], &frame, cam, &g2, 99)?;
                    visits.push(v);
                    let qn = q["name"].as_str().unwrap_or("sequence").to_string();
                    let sh = shot(&q["shot"], &frame, &game, Some(&qn))?;
                    Some((qn, g2, q["fps"].as_f64().unwrap_or(10.0), q["seconds"].as_f64().unwrap_or(done + 1.0), sh))
                }
                _ => None,
            };
            let weather_name = sv["weather"].as_str().unwrap_or("clear").to_string();
            scenes.push(Scene {
                name,
                hour: sv["time_of_day"].as_f64().unwrap_or(13.0),
                weather: eo_net::presence::weather_code(&weather_name).unwrap_or(0),
                weather_name,
                frame,
                camera_player: [back[0], back[1], frame.yaw],
                shots,
                gate,
                ready_s,
                players,
                sequence,
                input: sv.clone(),
            });
        }
        if scenes.is_empty() {
            anyhow::bail!("{}: no scenes", path.display());
        }
        let dir = args.out_dir.clone().ok_or_else(|| anyhow!("--net-shots needs --out DIR"))?;
        // Frames under the still camera: a still's, and each kept instant's of a sequence (the file may lower them on a busy GPU).
        let converge = file["converge_frames"].as_u64().map(|n| n as usize).unwrap_or(CONVERGE_FRAMES).max(8);
        let per_instant = file["frames_per_instant"].as_u64().map(|n| n as usize).unwrap_or(SEQUENCE_FRAMES).max(1);
        std::fs::create_dir_all(&dir)?;
        let pair = o.pair_before.clone();
        if let Some(b) = &pair {
            std::fs::create_dir_all(b)?;
        }
        // The server, its clock held still at the first scene's hour, and every scene's bots.
        let first = &scenes[0];
        let server = LocalServer::start(&world, &game, 0, [first.frame.o[0], first.frame.o[1]], o.server_sky(first.hour, 0.0, first.weather))?;
        let graph = Arc::new(RoadGraph::new(vec![]));
        let bots = BotRun::start(BotConfig { server: server.addr, count: visits.len(), threads: 1, spawn: eo_server::bots::Spawn::Spread, drive_share: 0.0, drive_mps: 0.0, seed: 5, conditions: Conditions::NONE, send_hz: 30.0, structures: game.structures, visits }, graph)?;
        let aspect = args.width as f32 / args.height as f32;
        let far = args.far.max(4000.0);
        let mut gpu: Option<(Offscreen, AgentRuntime, crate::props::PropsRuntime, eo_ui::Ui, TagProbe)> = None;
        let mut report = vec![];
        for (si, sc) in scenes.iter().enumerate() {
            server.ctl.lock().unwrap().sky = Some((sc.hour, 0.0, sc.weather, 1));
            let mut s = NetSession::connect(game.clone(), client_config(server.addr, &format!("camera{si}"), o, sc.camera_player, 100 + si as u64))?;
            s.presence = o.presence;
            s.draw_self = false;
            s.player_cars = false;
            wait_joined(&mut [&mut s], 180.0)?;
            // The first still's camera (or the sequence's, in a scene with no stills).
            let Some(first_shot) = sc.shots.first().or(sc.sequence.as_ref().map(|q| &q.4)) else { anyhow::bail!("scene {}: no shots and no sequence", sc.name) };
            let cam0 = camera(first_shot, aspect, far);
            if gpu.is_none() {
                let g = offscreen(&mut world, args, cam0.eye, process_start)?;
                let agents = AgentRuntime::start_remote(&world, args);
                let props = crate::props::PropsRuntime::start(&world, args);
                let land = eo_data::LandCover::load(&world.paths).ok();
                let mut ui = eo_ui::Ui::new(&g.ctx.device, &g.ctx.queue, Some(&world.paths.repo_root), None, Some(eo_ui::MapInputs { city: &world.city, tunnels: crate::structures::options(&world).is_some_and(|o| o.tunnels), land: land.as_ref(), backdrop_centre: [sc.frame.o[0] as f32, sc.frame.o[1] as f32] }));
                ui.set_screen(eo_ui::Screen::Hud);
                ui.hud_chrome = false;
                if let Some(w) = g.renderer.water.get() {
                    w.set_time(WATER_CLOCK_S);
                    w.set_always(true);
                }
                let probe = TagProbe::new(&g.ctx.device);
                gpu = Some((g, agents, props, ui, probe));
            }
            let (g, agents, props, ui, probe) = gpu.as_mut().unwrap();
            // The look: the scene's hour and weather for this client too (so `--no-shared-sky` draws the same sky).
            g.renderer.look.set_time_of_day(sc.hour);
            let base = args.look.sky;
            if base != eo_render::sky::SkyOptions::off() {
                let weather = match eo_sky::Kind::parse(&sc.weather_name) {
                    Some(k) => eo_render::sky::WeatherMode::Preset(k),
                    None => eo_render::sky::WeatherMode::Schedule,
                };
                g.renderer.look.set_sky_options(eo_render::sky::SkyOptions { weather, ..base });
            }
            g.renderer.look.sky.set_clock(base.world_time_s, 0.0);
            eprintln!("net-shots: scene {}: streaming in at ({:.0}, {:.0})", sc.name, cam0.eye.x, cam0.eye.z);
            let settle_ms = settle(g, &world, props, agents, &cam0, 600.0)?;
            // Run the scene: the bots walk up and settle into their emotes.
            sc.gate.store(true, Ordering::Relaxed);
            let t0 = Instant::now();
            let mut last = Instant::now();
            while t0.elapsed().as_secs_f64() < sc.ready_s + 1.0 {
                let dt = last.elapsed().as_secs_f64().min(0.1);
                last = Instant::now();
                frame(g, &world, props, agents, &mut s, &cam0, dt, true)?;
            }
            // The stills, from one frozen instant (and, with `--pair-before`, the
            // previous drawing of the same instant: the same street people and traffic).
            let tick = s.client.render_tick(Instant::now()).unwrap_or(0.0);
            s.poll();
            s.draw(agents, Some(tick), 0.0, false, DVec3::ZERO);
            s.apply_sky(&g.renderer, args.look.sky);
            let extras = agents.extras.clone();
            let remote = agents.remote.clone();
            let others = s.others.clone();
            let remap = agents.remap_people.clone();
            let before = pair.as_ref().map(|_| s.previous_extras(agents, Some(tick), 0.0, false));
            for sh in &sc.shots {
                let cam = camera(sh, aspect, far);
                agents.extras = extras.clone();
                agents.remote = remote.clone();
                agents.remap_people = remap.clone();
                let settle2 = settle(g, &world, props, agents, &cam, 600.0)?;
                agents.extras = extras.clone();
                for _ in 0..converge {
                    draw_frame(g, &world, props, agents, &cam, 0.0)?;
                }
                s.others = others.clone();
                let tags = tags_now(&mut s, g, probe, &cam, args, 0.0)?;
                let file = dir.join(format!("{}.png", sh.name));
                overlay_and_save(g, ui, &tags, &s.tag_avoid, args, &file)?;
                let entry = json!({
                    "name": sh.name, "scene": sc.name, "file": file.file_name().map(|f| f.to_string_lossy().to_string()),
                    "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "look_at": [sh.at.x, sh.at.y, sh.at.z], "fov_y_deg": sh.fov, "yaw_rad": cam.yaw, "pitch_rad": cam.pitch,
                    "server_tick": tick, "time_of_day": sc.hour, "weather": sc.weather_name, "sky_drawn": s.sky_shown.map(|k| json!({ "hour": k.hour, "weather": eo_net::presence::weather_name(k.weather) })),
                    "players": sc.players, "others_drawn": s.others.len(), "name_tags": tags.len(),
                    "tags": tag_report(&s, &tags),
                    "street_people_drawn": remote.as_ref().map(|f| f.regions.iter().map(|r| r.walkers.len()).sum::<usize>()), "cars_drawn": remote.as_ref().map(|f| f.regions.iter().map(|r| r.vehicles.len()).sum::<usize>()),
                    "presence": { "remote_bodies": o.presence.bodies, "name_tags": o.presence.tags, "shared_sky": o.presence.shared_sky },
                    "player_cars_drawn": false, "converge_frames": converge, "settle_ms": settle_ms + settle2,
                });
                std::fs::write(dir.join(format!("{}.json", sh.name)), serde_json::to_vec_pretty(&entry)?)?;
                report.push(entry.clone());
                println!("wrote {}", file.display());
                if let (Some(bdir), Some(bx)) = (pair.as_ref(), before.as_ref()) {
                    // The same instant, the previous drawing (`--no-remote-bodies --no-name-tags`).
                    agents.extras = bx.clone();
                    agents.remote = remote.clone();
                    agents.remap_people.clear();
                    for _ in 0..converge {
                        draw_frame(g, &world, props, agents, &cam, 0.0)?;
                    }
                    let bfile = bdir.join(format!("{}.png", sh.name));
                    overlay_and_save(g, ui, &[], &[], args, &bfile)?;
                    let mut be = entry.clone();
                    be["file"] = json!(bfile.file_name().map(|f| f.to_string_lossy().to_string()));
                    be["presence"] = json!({ "remote_bodies": false, "name_tags": false, "shared_sky": o.presence.shared_sky });
                    be["name_tags"] = json!(0);
                    be["tags"] = json!([]);
                    be["pair_of"] = json!(file.display().to_string());
                    std::fs::write(bdir.join(format!("{}.json", sh.name)), serde_json::to_vec_pretty(&be)?)?;
                }
            }
            // The sequence: one more player walks up to the camera and stops, the world running.
            if let Some((name, gate, fps, seconds, sh)) = &sc.sequence {
                let cam = camera(sh, aspect, far);
                agents.extras.clear();
                settle(g, &world, props, agents, &cam, 600.0)?;
                // Recorded at wall time with the world running (the client draws every
                // 4 ms, as a fast frame loop would), every 1/fps s the instant kept;
                // then each kept instant drawn under the still camera until it resolves
                // (every kept instant of the sequence in turn, then the previous
                // drawing's, each pass its own run of frames).
                gate.store(true, Ordering::Relaxed);
                let t0 = Instant::now();
                let mut last = Instant::now();
                let mut next = 0.0;
                let mut kept = vec![];
                let mut log = vec![];
                while t0.elapsed().as_secs_f64() < *seconds {
                    let dt = last.elapsed().as_secs_f64().min(0.1);
                    last = Instant::now();
                    s.poll();
                    s.step(&StepInput::default(), dt);
                    s.draw(agents, None, dt, false, DVec3::ZERO);
                    let on = agents.extras.clone();
                    let off = pair.as_ref().map(|_| s.previous_extras(agents, None, dt, false));
                    if t0.elapsed().as_secs_f64() >= next {
                        log.push(json!({ "frame": kept.len(), "wall_s": t0.elapsed().as_secs_f64(), "server_tick": s.tick, "others": s.others.iter().map(|p| json!({ "id": p.id, "name": s.names.get(&p.id), "feet": p.feet, "speed": p.speed, "emote": p.emote.name() })).collect::<Vec<_>>() }));
                        kept.push((on, off, agents.remote.clone(), s.others.clone(), agents.remap_people.clone()));
                        next += 1.0 / fps;
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
                let k = kept.len();
                let sdir = dir.join(name);
                std::fs::create_dir_all(&sdir)?;
                let mut tag_log = vec![];
                for (i, (on, _, remote, others, remap)) in kept.iter().enumerate() {
                    agents.extras = on.clone();
                    agents.remote = remote.clone();
                    agents.remap_people = remap.clone();
                    for _ in 0..per_instant {
                        draw_frame(g, &world, props, agents, &cam, 0.0)?;
                    }
                    s.others = others.clone();
                    if let Some(f) = remote {
                        // The street people of that instant (the faces the tags keep off).
                        s.frame = f.clone();
                    }
                    let tags = tags_now(&mut s, g, probe, &cam, args, if i == 0 { 0.0 } else { 1.0 / fps })?;
                    tag_log.push(json!({ "frame": i, "tags": tag_report(&s, &tags) }));
                    overlay_and_save(g, ui, &tags, &s.tag_avoid, args, &sdir.join(format!("{i:04}.png")))?;
                }
                if let Some(bdir) = pair.as_ref() {
                    let bsdir = bdir.join(name);
                    std::fs::create_dir_all(&bsdir)?;
                    for (i, (_, off, remote, _, _)) in kept.iter().enumerate() {
                        agents.extras = off.clone().unwrap_or_default();
                        agents.remote = remote.clone();
                        agents.remap_people.clear();
                        for _ in 0..per_instant {
                            draw_frame(g, &world, props, agents, &cam, 0.0)?;
                        }
                        overlay_and_save(g, ui, &[], &[], args, &bsdir.join(format!("{i:04}.png")))?;
                    }
                }
                let m = json!({ "name": name, "scene": sc.name, "fps": fps, "seconds": seconds, "frames": k, "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "look_at": [sh.at.x, sh.at.y, sh.at.z], "presence": { "remote_bodies": o.presence.bodies, "name_tags": o.presence.tags }, "log": log, "tags": tag_log, "note": format!("the world run at wall time (the client drawing every 4 ms), the other players, traffic and tags kept every 1/fps s, each kept instant then drawn {per_instant} frames under the still camera; with --pair-before the previous drawing of the same instants after") });
                std::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec_pretty(&m)?)?;
                println!("wrote {} frames to {}", k, sdir.display());
            }
            report.push(json!({ "scene": sc.name, "input": sc.input, "corrections": s.corrections, "names": s.names }));
            drop(s);
        }
        std::fs::write(dir.join("net-shots.json"), serde_json::to_vec_pretty(&json!({ "file": path.display().to_string(), "command_line": std::env::args().collect::<Vec<_>>(), "build": env!("EO_BUILD_COMMIT"), "shots": report }))?)?;
        let _ = bots.stop();
        drop(server);
        Ok(())
    }

    /// The name tags over the frame just drawn: their heads' depth probed
    /// against it (waited for), then the tags (easing over `dt`).
    fn tags_now(s: &mut NetSession, g: &mut Offscreen, probe: &mut TagProbe, cam: &Camera, args: &Args, dt: f64) -> Result<Vec<eo_ui::NameTag>> {
        // The probe's asks for this camera, the tags' easing left as it was.
        let eased = s.tag_alpha.clone();
        s.name_tags(cam, args.width, args.height, 0.0);
        s.tag_alpha = eased;
        let asks = s.probe_asks.clone();
        probe.run_now(&g.ctx, &g.targets.depth, &asks, &mut s.head_visible)?;
        Ok(s.name_tags(cam, args.width, args.height, dt))
    }

    /// What the tags were, for a capture's manifest.
    fn tag_report(s: &NetSession, tags: &[eo_ui::NameTag]) -> Vec<Value> {
        let by_name: HashMap<String, u32> = s.names.iter().map(|(k, v)| (v.clone(), *k)).collect();
        tags.iter().map(|t| json!({ "name": t.name, "detail": t.detail, "alpha": t.alpha, "anchor": [t.x, t.y], "depth_m": t.depth, "size_px": t.size, "head_visible": by_name.get(&t.name).and_then(|id| s.head_visible.get(id)) })).collect()
    }

    /// One running frame: the network, the camera player standing still, the world streamed and drawn.
    #[allow(clippy::too_many_arguments)]
    fn frame(g: &mut Offscreen, world: &World, props: &mut crate::props::PropsRuntime, agents: &mut AgentRuntime, s: &mut NetSession, cam: &Camera, dt: f64, _warm: bool) -> Result<()> {
        s.poll();
        s.step(&StepInput::default(), dt);
        s.draw(agents, None, dt, false, DVec3::ZERO);
        s.apply_sky(&g.renderer, eo_render::sky::SkyOptions { ..g.renderer.look.sky.options() });
        draw_frame(g, world, props, agents, cam, dt)
    }

    fn draw_frame(g: &mut Offscreen, world: &World, props: &mut crate::props::PropsRuntime, agents: &mut AgentRuntime, cam: &Camera, dt: f64) -> Result<()> {
        g.models.set_camera(cam.eye);
        g.models.pump(&g.ctx, &g.renderer, world, cam.eye)?;
        props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, cam.eye, None, dt);
        if props.on {
            agents.parked = props.parked_agents(agents.lib.as_deref(), cam.eye).to_vec();
        }
        agents.pump(&g.ctx, &g.renderer, [cam.eye.x as f64, cam.eye.z as f64, 0.0], dt);
        g.renderer.update(&g.ctx.queue, cam);
        let mut enc = g.ctx.device.create_command_encoder(&Default::default());
        g.renderer.encode(&mut enc, &g.view, None, &g.targets.depth, None);
        g.ctx.queue.submit([enc.finish()]);
        g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
        Ok(())
    }

    /// The name tags over the drawn frame (the HUD pass, plates off), then the PNG.
    fn overlay_and_save(g: &mut Offscreen, ui: &mut eo_ui::Ui, tags: &[eo_ui::NameTag], avoid: &[[f32; 4]], args: &Args, file: &std::path::Path) -> Result<()> {
        if !tags.is_empty() {
            ui.tags = tags.to_vec();
            ui.tag_avoid = avoid.to_vec();
            ui.set_time(100.0);
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, args.width, args.height, &eo_ui::Input::default());
            ui.draw(&g.ctx.device, &mut enc, &g.unorm, wgpu::TextureFormat::Rgba8Unorm, None);
            g.ctx.queue.submit([enc.finish()]);
        }
        save_png(&g.ctx, &g.color, file)
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn parsed(flags: &[&str]) -> bool {
        let raw: Vec<String> = ["--connect", "local"].iter().chain(flags).map(|s| s.to_string()).collect();
        parse(&raw).expect("the online arguments parse").0.props2.poster_hoardings
    }

    /// Online launches take the poster hoarding switch as `--play` does: off by default,
    /// `--poster-hoardings` draws them, a later `--no-poster-hoardings` turns them off again.
    #[test]
    fn poster_hoarding_switch_online() {
        assert!(!parsed(&[]));
        assert!(parsed(&["--poster-hoardings"]));
        assert!(!parsed(&["--poster-hoardings", "--no-poster-hoardings"]));
    }
}
