//! the live simulation (eo-sim's `AsyncSim`, on its own thread),
//! the agent feed (eo-agents, its own thread: grounding, bodies, paint,
//! clips) and the agent library (loaded on a thread at start), driven from
//! the render thread once a frame: post the frame's time and the focus (the
//! camera or the player), pick up the newest instances, upload them.
//!
//! `--no-agents` turns all of it off; `--agent-density N` multiplies the
//! default population (42 moving cars, People `medium`) N times in one region.
//!
//! The corridor layer (`corridor`, eo-sim `corridor`) adds the long roads'
//! through traffic and the bridge's walkers on the same clock;
//! `--no-corridor-traffic` leaves it out.

mod corridor;

use crate::{Args, World};
use anyhow::Result;
use eo_agents::GpuAgent;
use eo_agents::feed::{FeedThread, Feeder, Ground};
use eo_agents::library::Library;
use eo_render::{GpuContext, Renderer};
use eo_sim::region::{Focus, People, Region, RegionConfig};
use eo_sim::{AsyncSim, Network, World as SimWorld};
use glam::Vec3;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Instant;

/// The simulation's resident network: tiles within this of the focus (as `sim_bench`).
pub const SIM_RADIUS_M: f64 = 600.0;
/// Re-stream the network when the focus has moved this far.
const RESTREAM_M: f64 = 32.0;
/// Fixed steps run before the first frame so the population has filled in.
const WARMUP_STEPS: usize = 600;
pub const SEED_INT: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct AgentOpts {
    pub on: bool,
    pub density: u32,
}

impl Default for AgentOpts {
    fn default() -> Self {
        Self { on: true, density: 1 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AgentFrameStats {
    /// Render thread: post + pick up + compose + upload, ms.
    pub cpu_ms: f64,
    pub uploaded: usize,
    pub upload_bytes: usize,
    /// The feed thread's last build, ms.
    pub feed_ms: f64,
}

pub struct AgentRuntime {
    pub opts: AgentOpts,
    pub lib: Option<Arc<Library>>,
    lib_rx: Option<Receiver<Result<Library, String>>>,
    sim_rx: Option<Receiver<Result<(SimWorld, Arc<eo_sim::native::NativeSource>), String>>>,
    /// The corridor layer's hour (written from the look's time of day each pump).
    corridor_hour: Option<Arc<AtomicU64>>,
    /// The look's time of day at the last pump (the hour a rebuilt corridor layer starts at).
    last_hour: f64,
    /// The Golden Gate deck's ground grid (the drawn terrain under it), for the corridor layer.
    gg_ground: Option<Arc<eo_landmarks::GroundGrid>>,
    pub sim: Option<AsyncSim>,
    feed: Option<FeedThread>,
    ground: Ground,
    last: Option<Arc<Vec<GpuAgent>>>,
    last_steps: u64,
    scratch: Vec<GpuAgent>,
    /// Instances the caller adds this frame (the player and the player's car).
    pub extras: Vec<GpuAgent>,
    /// the parked cars near the camera (static instances, set each frame).
    pub parked: Vec<GpuAgent>,
    pub started: Instant,
    pub sim_ready_ms: Option<f64>,
    pub lib_ready_ms: Option<f64>,
    pub errors: Vec<String>,
    pub sim_time: f64,
    /// the frame to draw this frame, posted by the online client (`--connect`) instead of a local simulation.
    pub remote: Option<eo_sim::world::Frame>,
    /// leave the simulated people out of the upload (the people pass A/B; extras stay).
    pub hide_people: bool,
    remote_mode: bool, // net
    /// groundscale (captures only, empty otherwise): simulated agents whose bounding
    /// radius comes within these (point, horizontal margin) of the point are not drawn
    /// (a traffic car driven through the player's car or the lens; the simulation does
    /// not see the player's car).
    pub hide_near: Vec<([f32; 3], f32)>,
}

/// One region around `focus` at `density` times the default population.
fn region_config(density: u32) -> RegionConfig {
    let mut cfg = RegionConfig::game_default(SEED_INT);
    let n = density.max(1) as usize;
    cfg.vehicle_pool *= n;
    if let People::Bubble(b) = &mut cfg.people {
        b.max_bodies *= n;
    }
    cfg
}

impl AgentRuntime {
    /// Start loading (library, simulation network) around `focus` = (x, z, yaw).
    pub fn start(world: &World, args: &Args, focus: [f64; 3]) -> Self {
        Self::start_mode(world, args, focus, false)
    }

    /// the library and the feed only; the frames come from the server (`remote`).
    pub fn start_remote(world: &World, args: &Args) -> Self {
        Self::start_mode(world, args, [0.0; 3], true)
    }

    fn start_mode(world: &World, args: &Args, focus: [f64; 3], remote_mode: bool) -> Self {
        let opts = AgentOpts { on: args.agents, density: args.agent_density };
        let ground = match &world.street {
            Some(s) => Ground::Street(s.world.clone()),
            None => Ground::Elevation(Arc::new(world.elevation.clone())),
        };
        let mut rt = AgentRuntime {
            opts,
            lib: None,
            lib_rx: None,
            sim_rx: None,
            corridor_hour: None,
            last_hour: f64::NAN,
            gg_ground: None,
            sim: None,
            feed: None,
            ground,
            last: None,
            last_steps: 0,
            scratch: vec![],
            extras: vec![],
            parked: vec![], // props
            hide_near: vec![],
            started: Instant::now(),
            sim_ready_ms: None,
            lib_ready_ms: None,
            errors: vec![],
            sim_time: 0.0,
            remote: None,  // net
            remote_mode,
            hide_people: false, // people
        };
        if !opts.on {
            return rt;
        }
        let repo = world.paths.repo_root.clone();
        let cache = crate::cache_dir(args);
        // the Rocketbox people unless `--no-rocketbox` (or no copy at EO_PEOPLE_ROOT): then none.
        let people = if args.rocketbox { eo_agents::library::PeopleSource::from_env() } else { eo_agents::library::PeopleSource::Off };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-load".into())
            .spawn(move || {
                let _ = tx.send(Library::load_with(&repo, cache.as_deref(), &people).map_err(|e| format!("{e:#}")));
            })
            .expect("spawn agent library loader");
        rt.lib_rx = Some(rx);
        if !remote_mode {
            rt.restart_sim(world, focus, opts.density);
        }
        rt
    }

    /// (Re)build the simulation around `focus` at a population density.
    pub fn restart_sim(&mut self, world: &World, focus: [f64; 3], density: u32) {
        self.sim = None;
        self.feed = None;
        self.last = None;
        self.last_steps = 0;
        self.sim_ready_ms = None;
        self.opts.density = density;
        self.corridor_hour = None;
        let paths = world.paths.clone();
        let (traffic, walkers) = corridor::switches();
        if traffic && self.gg_ground.is_none() {
            let js = eo_landmarks::LandmarksJs::get();
            self.gg_ground = Some(Arc::new(eo_landmarks::set::ground_of(js, "golden-gate-bridge", &|x: f64, z: f64| world.elevation.sample(x, z))));
        }
        let gg_ground = self.gg_ground.clone().filter(|_| traffic);
        let ground = self.ground.clone();
        let hour0 = if self.last_hour.is_finite() { self.last_hour } else { 12.0 };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-sim-build".into())
            .spawn(move || {
                let r = (|| -> Result<(SimWorld, Arc<eo_sim::native::NativeSource>)> {
                    let mut source = eo_sim::native::load(&paths)?;
                    // the corridor layer's ways, read before the local network leaves the roads it drives alone.
                    let corridors = match &gg_ground {
                        Some(gg) => Some(build_corridors(&paths, &source, gg, &ground, walkers, hour0)?),
                        None => None,
                    };
                    if corridors.is_some() {
                        source.without_roads(|r| corridor::EXCLUSIVE.contains(&r.name.as_str()));
                    }
                    let tiles = eo_sim::native::tiles_around(focus[0], focus[1], SIM_RADIUS_M);
                    let mut net = Network::new(source.timing.clone());
                    net.add(source.build(&tiles));
                    let mut w = SimWorld::new(net);
                    w.corridors = corridors;
                    let f = Focus { x: focus[0], z: focus[1], yaw: focus[2] };
                    let r = Region::new(&w.net, 0, region_config(density), f);
                    w.regions.push(r);
                    for _ in 0..WARMUP_STEPS {
                        w.step();
                    }
                    Ok((w, Arc::new(source)))
                })();
                let _ = tx.send(r.map_err(|e| format!("{e:#}")));
            })
            .expect("spawn simulation builder");
        self.sim_rx = Some(rx);
    }

    /// Everything loaded and the first instances built.
    pub fn ready(&self) -> bool {
        !self.opts.on || (self.lib.is_some() && (self.sim.is_some() || self.remote_mode) && self.last.is_some()) // or remote
    }

    /// Wait (pumping) until ready or `timeout_s`.
    pub fn wait_ready(&mut self, ctx: &GpuContext, renderer: &Renderer, focus: [f64; 3], timeout_s: f64) -> bool {
        let t = Instant::now();
        while !self.ready() && t.elapsed().as_secs_f64() < timeout_s {
            self.pump(ctx, renderer, focus, 1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.ready()
    }

    /// Once a frame on the render thread, before `Renderer::update`.
    pub fn pump(&mut self, ctx: &GpuContext, renderer: &Renderer, focus: [f64; 3], dt: f64) -> AgentFrameStats {
        let t0 = Instant::now();
        let mut st = AgentFrameStats::default();
        if let Some(rx) = &self.lib_rx {
            if let Ok(r) = rx.try_recv() {
                self.lib_rx = None;
                match r {
                    Ok(lib) => {
                        if let Err(e) = renderer.attach_agents(ctx, &lib) {
                            self.errors.push(format!("agent renderer: {e:#}"));
                            eprintln!("agents: renderer failed: {e:#}");
                        }
                        self.lib_ready_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                        self.lib = Some(Arc::new(lib));
                    }
                    Err(e) => {
                        eprintln!("agents: library failed: {e}");
                        self.errors.push(e);
                    }
                }
            }
        }
        if let Some(rx) = &self.sim_rx {
            if let Ok(r) = rx.try_recv() {
                self.sim_rx = None;
                match r {
                    Ok((w, src)) => {
                        self.corridor_hour = w.corridors.as_ref().map(|c| c.hour_handle());
                        let source = Some(src);
                        let mut last = [f64::NAN; 2];
                        self.sim = Some(AsyncSim::start_with(w, move |w| {
                            let Some(src) = &source else { return };
                            let f = w.regions.first().map(|r| [r.focus.x, r.focus.z]).unwrap_or([0.0; 2]);
                            if !((f[0] - last[0]).hypot(f[1] - last[1]) < RESTREAM_M) {
                                last = f;
                                eo_sim::native::stream(w, src, SIM_RADIUS_M);
                            }
                        }));
                        self.sim_ready_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                    }
                    Err(e) => {
                        eprintln!("agents: simulation failed: {e}");
                        self.errors.push(e);
                    }
                }
            }
        }
        if self.feed.is_none() && (self.sim.is_some() || self.remote_mode) {
            // a remote runtime starts its feed without a local simulation
            if let Some(lib) = &self.lib {
                match Feeder::new(lib.clone(), self.ground.clone(), SEED_INT) {
                    Ok(f) => self.feed = Some(FeedThread::start(f)),
                    Err(e) => self.errors.push(format!("{e:#}")),
                }
            }
        }
        let gpu = renderer.agents.get();
        if let Some(g) = gpu {
            g.set_enabled(self.opts.on);
        }
        self.last_hour = renderer.look.time_of_day();
        if let Some(h) = &self.corridor_hour {
            h.store(self.last_hour.to_bits(), Ordering::Relaxed);
        }
        if let Some(sim) = &mut self.sim {
            sim.post(dt, &[(0, Focus { x: focus[0], z: focus[1], yaw: focus[2] })]);
            let fr = sim.latest();
            self.sim_time = fr.time;
            if fr.steps != self.last_steps {
                self.last_steps = fr.steps;
                if let Some(feed) = &self.feed {
                    feed.post(fr.clone());
                }
            }
        } else if let Some(fr) = self.remote.take() {
            // the server's world, interpolated by the online client.
            self.sim_time = fr.time;
            if let Some(feed) = &self.feed {
                feed.post(fr);
            }
        }
        if let Some(feed) = &self.feed {
            let (out, ms) = feed.take();
            if let Some(o) = out {
                self.last = Some(o);
            }
            st.feed_ms = ms;
        }
        if let Some(g) = gpu {
            if self.opts.on {
                let base: &[GpuAgent] = self.last.as_deref().map(|v| v.as_slice()).unwrap_or(&[]);
                // without the simulated people (the people pass A/B).
                let filtered: Vec<GpuAgent>;
                let base: &[GpuAgent] = if self.hide_people {
                    let lib = self.lib.clone();
                    filtered = base.iter().copied().filter(|a| lib.as_ref().is_none_or(|l| l.models.get(a.model as usize).is_none_or(|m| m.class != eo_agents::CLASS_PERSON))).collect();
                    &filtered
                } else {
                    base
                };
                let list: &[GpuAgent] = if self.extras.is_empty() && self.parked.is_empty() && self.hide_near.is_empty() { // + parked; + hide_near
                    base
                } else {
                    self.scratch.clear();
                    if self.hide_near.is_empty() {
                        self.scratch.extend_from_slice(base);
                    } else {
                        // captures only. An agent is hidden when its body's
                        // bounding radius comes within `r` of a point (a bus as well as a car).
                        let hide = &self.hide_near;
                        let lib = self.lib.as_deref();
                        let body = |a: &GpuAgent| lib.and_then(|l| l.models.get(a.model as usize)).map(|m| m.radius * a.scale.max(1.0)).unwrap_or(3.0);
                        self.scratch.extend(base.iter().filter(|a| !hide.iter().any(|(p, r)| (a.pos[0] - p[0]).hypot(a.pos[2] - p[2]) < *r + body(a))));
                    }
                    self.scratch.extend_from_slice(&self.extras);
                    self.scratch.extend_from_slice(&self.parked); // props
                    &self.scratch
                };
                st.uploaded = g.upload(&ctx.queue, list);
                st.upload_bytes = st.uploaded * std::mem::size_of::<GpuAgent>();
            }
        }
        st.cpu_ms = t0.elapsed().as_secs_f64() * 1e3;
        st
    }

    /// Walkers the feed moved off a drawn carriageway onto its footway (last build).
    pub fn moved_to_footway(&self) -> usize {
        self.feed.as_ref().map(|f| f.moved_to_footway()).unwrap_or(0)
    }

    /// The newest simulated instances (captures pick their subjects from them).
    pub fn snapshot(&self) -> Vec<GpuAgent> {
        self.last.as_deref().cloned().unwrap_or_default()
    }

    /// The player's body and cars as this frame's extra instances (`--play`).
    pub fn set_player(&mut self, visual: &mut PlayerVisual, player: &eo_player::Player, cw: &eo_player::world::CollisionWorld, body_visible: bool, dt: f64) {
        self.extras.clear();
        if let Some(lib) = &self.lib {
            visual.instances(lib, player, cw, body_visible, dt, &mut self.extras);
        }
    }

    /// The camera as the simulation's focus: (x, z, view yaw atan2(dx, dz)).
    pub fn focus_of(eye: Vec3, forward: Vec3) -> [f64; 3] {
        [eye.x as f64, eye.z as f64, (forward.x as f64).atan2(forward.z as f64)]
    }
}

/// The corridor layer on the drawn deck and streets (`corridor`), starting at `hour`.
fn build_corridors(paths: &eo_data::DataPaths, source: &eo_sim::native::NativeSource, gg: &eo_landmarks::GroundGrid, ground: &Ground, walkers: bool, hour: f64) -> Result<eo_sim::corridor::Corridors> {
    let js = eo_landmarks::LandmarksJs::get();
    let ca = eo_data::CaStandards::load(paths)?;
    let k_width = eo_landmarks::suspension::DeckStandards::load(&ca)?.k_width;
    let time_gap = eo_sim::Standards::load(paths)?.metric("following-time-gap")?;
    let site = eo_landmarks::golden_gate::Site::new(&eo_landmarks::golden_gate::data(js));
    let deck = corridor::Deck::new(js, &site, &|x: f64, z: f64| gg.at(x, z), k_width);
    let paved = |x: f64, z: f64| match ground {
        Ground::Street(w) => w.paved_heights(x, z),
        _ => vec![],
    };
    let carpet = |x: f64, z: f64| ground.y(x, z);
    let surfaces = corridor::Surfaces { deck: Some(&deck), paved: &paved, ground: &carpet };
    let ways = corridor::ways(source, &surfaces, walkers);
    let c = eo_sim::corridor::Corridors::new(ways, eo_sim::corridor::Params::with_time_gap(time_gap), SEED_INT);
    c.hour_handle().store(hour.to_bits(), Ordering::Relaxed);
    Ok(c)
}

/// The player's drawn state between frames: the walk phase (`advanceGaitPhase`;
/// eo-player does not phase the body) and each car's wheel spin.
#[derive(Default)]
pub struct PlayerVisual {
    pub phase: f64,
    pub spins: Vec<f64>,
    pub time: f64,
}

/// The player's car body (src/vehicles/hero-sedan-glb.js `HERO_SEDAN_ASSET`: the sedan LOD0).
pub const PLAYER_CAR_BODY: &str = "sedan";
/// The player's car paint (the civilian set's `red`; the JS hero sedan takes its paint from the presentation draw).
pub const PLAYER_CAR_PAINT: &str = "#a8332b";

impl PlayerVisual {
    pub fn instances(&mut self, lib: &Library, player: &eo_player::Player, cw: &eo_player::world::CollisionWorld, body_visible: bool, dt: f64, out: &mut Vec<GpuAgent>) {
        use eo_player::Mode;
        self.time += dt;
        let height = lib.meta.family_heights.first().copied().unwrap_or(1.75) as f64;
        self.phase = eo_agents::gait::advance_phase(self.phase, player.ground_speed, dt, height);
        if player.mode == Mode::OnFoot && body_visible {
            let f = player.character.feet();
            if let Some(b) = eo_agents::feed::player_body(lib, [f.x, f.y, f.z], player.facing_yaw, player.ground_speed, self.phase, self.time) {
                out.push(b);
            }
        }
        self.spins.resize(player.vehicles.len(), 0.0);
        for (k, v) in player.vehicles.iter().enumerate() {
            let speed = v.forward_speed(cw);
            self.spins[k] += speed * dt / v.spec.wheel_radius.max(0.2);
            let (_, rot) = v.pose(cw);
            let datum = v.datum(cw);
            let steer = v.controller.wheels().first().map(|w| w.steering as f64).unwrap_or(0.0);
            let braking = v.controls.brake > 0.0 || v.controls.handbrake;
            if let Some(c) = eo_agents::feed::player_car(lib, PLAYER_CAR_BODY, [datum.x, datum.y, datum.z], [rot.x, rot.y, rot.z, rot.w], self.spins[k], steer, speed, braking, PLAYER_CAR_PAINT) {
                out.push(c);
            }
        }
    }
}
