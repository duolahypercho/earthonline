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
    /// `--car-detail` (0 as before): the library's far bodies and the renderer's vehicle tiers.
    pub car_detail: u8,
}

impl Default for AgentOpts {
    fn default() -> Self {
        Self { on: true, density: 1, car_detail: 0 }
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
    lib_rx: Option<Receiver<Result<(Library, Option<String>), String>>>,
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
    /// the grade-separated roads beside the ground (`--structures`), handed to the feed.
    structures: Option<Arc<eo_structures::StructureWorld>>,
    last: Option<Arc<Vec<GpuAgent>>>,
    last_steps: u64,
    last_corridor_steps: u64,
    scratch: Vec<GpuAgent>,
    /// Instances the caller adds this frame (the player and the player's car).
    pub extras: Vec<GpuAgent>,
    test_rows: Vec<[f32; 4]>,
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
    /// `--smooth-time`: the drawn hour as anchored to the simulation's clock
    /// (the look's hour then, the simulation time then, the hour last set).
    clock: Option<(f64, f64, f64)>,
    /// groundscale (captures only, empty otherwise): simulated agents whose bounding
    /// radius comes within these (point, horizontal margin) of the point are not drawn
    /// (a traffic car driven through the player's car or the lens; the simulation does
    /// not see the player's car).
    pub hide_near: Vec<([f32; 3], f32)>,
    /// online (`net.rs`): street people drawn in one of these bodies are
    /// drawn in the other (from, to) instead, so no passer-by wears a
    /// player's body (empty: as simulated).
    pub remap_people: Vec<(u32, u32)>,
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
        let opts = AgentOpts { on: args.agents, density: args.agent_density, car_detail: args.car_detail };
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
            structures: world.structures.clone(),
            last: None,
            last_steps: 0,
            last_corridor_steps: 0,
            scratch: vec![],
            extras: vec![],
            test_rows: vec![],
            parked: vec![], // props
            hide_near: vec![],
            remap_people: vec![],
            started: Instant::now(),
            sim_ready_ms: None,
            lib_ready_ms: None,
            errors: vec![],
            sim_time: 0.0,
            remote: None,  // net
            remote_mode,
            hide_people: false, // people
            clock: None, // smooth time
        };
        if !opts.on {
            return rt;
        }
        let repo = world.paths.repo_root.clone();
        let cache = crate::cache_dir(args);
        let test_character = args.test_character.clone();
        // the Rocketbox people unless `--no-rocketbox` (or no copy in the people folder: external/people or EO_PEOPLE_ROOT): then none.
        let people = if args.rocketbox { eo_agents::library::PeopleSource::from_env() } else { eo_agents::library::PeopleSource::Off };
        // (`--car-detail`) each body's far slot its own simplified body instead of three boxes.
        let cook = eo_agents::library::CookOptions { far_body: opts.car_detail > 0 };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-load".into())
            .spawn(move || {
                let loaded = Library::load_opts(&repo, cache.as_deref(), &people, cook).map(|mut lib| {
                    let issue = test_character.as_ref().and_then(|selection| {
                        lib.install_test_player(selection).err().map(|e| format!("test character {}: {e:#}", selection.id))
                    });
                    (lib, issue)
                });
                let _ = tx.send(loaded.map_err(|e| format!("{e:#}")));
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
        self.last_corridor_steps = 0;
        self.sim_ready_ms = None;
        self.opts.density = density;
        self.corridor_hour = None;
        let paths = world.paths.clone();
        // The structure level of each way (`--structures`; empty otherwise, which leaves the sim as it was).
        let levels = self.structures.as_ref().map(|s| s.way_levels().clone()).unwrap_or_default();
        let (traffic, walkers) = corridor::switches();
        if traffic && self.gg_ground.is_none() {
            let js = eo_landmarks::LandmarksJs::get();
            self.gg_ground = Some(Arc::new(eo_landmarks::set::ground_of(js, "golden-gate-bridge", &|x: f64, z: f64| world.elevation.sample(x, z))));
        }
        let gg_ground = self.gg_ground.clone().filter(|_| traffic);
        let ground = self.ground.clone();
        // the deck the traffic rides is the drawn bridge's (none with `--no-landmarks`).
        let deck_drawn = world.landmarks.is_some();
        let hour0 = if self.last_hour.is_finite() { self.last_hour } else { 12.0 };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-sim-build".into())
            .spawn(move || {
                let r = (|| -> Result<(SimWorld, Arc<eo_sim::native::NativeSource>)> {
                    let mut source = eo_sim::native::load(&paths)?.with_levels(levels);
                    // the corridor layer's ways, read before the local network leaves the roads it drives alone.
                    let corridors = match &gg_ground {
                        Some(gg) => Some(build_corridors(&paths, &source, gg, &ground, walkers && deck_drawn, deck_drawn, hour0)?),
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
                    Ok((lib, issue)) => {
                        if let Some(issue) = issue {
                            eprintln!("agents: {issue}");
                            self.errors.push(issue);
                        }
                        if let Some(body) = &lib.test_player {
                            eprintln!("agents: loaded test character {} with {} clips", lib.meta.model_names[body.model as usize], body.rig.clip_count());
                        }
                        if let Err(e) = renderer.attach_agents_with(ctx, &lib, self.opts.car_detail) {
                            self.errors.push(format!("agent renderer: {e:#}"));
                            eprintln!("agents: renderer failed: {e:#}");
                        }
                        // (`--tunnels`) the cars in a tunnel's bore lit by the bore: their pipelines
                        // (`EO_AGENTS_NO_BORE_LIGHT=1`, a diagnostic: lit by the sun and sky as before, the evidence's other side).
                        if let (Some(a), true) = (renderer.agents.get(), self.structures.as_ref().is_some_and(|s| !s.bores().is_empty())) {
                            if std::env::var_os("EO_AGENTS_NO_BORE_LIGHT").is_none() {
                                a.enable_bores();
                            }
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
                match Feeder::new(lib.clone(), self.ground.clone(), self.structures.clone(), SEED_INT) {
                    Ok(f) => self.feed = Some(FeedThread::start(f)),
                    Err(e) => self.errors.push(format!("{e:#}")),
                }
            }
        }
        let gpu = renderer.agents.get();
        if let Some(g) = gpu {
            g.set_enabled(self.opts.on);
        }
        let mut clock_read = false; // smooth time
        self.last_hour = renderer.look.time_of_day();
        if let Some(h) = &self.corridor_hour {
            h.store(self.last_hour.to_bits(), Ordering::Relaxed);
        }
        if let Some(sim) = &mut self.sim {
            sim.post(dt, &[(0, Focus { x: focus[0], z: focus[1], yaw: focus[2] })]);
            let fr = sim.latest();
            self.sim_time = fr.time;
            clock_read = true;
            // a new frame: the regions or the corridor layer stepped (it keeps its own count).
            if fr.steps != self.last_steps || fr.corridor_steps != self.last_corridor_steps {
                self.last_steps = fr.steps;
                self.last_corridor_steps = fr.corridor_steps;
                if let Some(feed) = &self.feed {
                    feed.post(fr.clone());
                }
            }
        } else if let Some(fr) = self.remote.take() {
            // the server's world, interpolated by the online client.
            self.sim_time = fr.time;
            clock_read = true;
            if let Some(feed) = &self.feed {
                feed.post(fr);
            }
        }
        if clock_read {
            self.follow_clock(renderer);
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
                let list: &[GpuAgent] = if self.extras.is_empty() && self.parked.is_empty() && self.hide_near.is_empty() && self.remap_people.is_empty() { // + parked; + hide_near; + remap_people
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
                    if !self.remap_people.is_empty() {
                        for a in self.scratch.iter_mut() {
                            if let Some(&(_, to)) = self.remap_people.iter().find(|(from, _)| *from == a.model) {
                                a.model = to;
                            }
                        }
                    }
                    self.scratch.extend_from_slice(&self.extras);
                    self.scratch.extend_from_slice(&self.parked); // props
                    &self.scratch
                };
                st.uploaded = g.upload(&ctx.queue, list);
                st.upload_bytes = st.uploaded * std::mem::size_of::<GpuAgent>()
                    + g.upload_test_player(&ctx.queue, &self.test_rows);
            }
        }
        st.cpu_ms = t0.elapsed().as_secs_f64() * 1e3;
        st
    }

    /// `--smooth-time`: the drawn hour follows the simulation's clock (eo-sim's
    /// fixed-step `World::time`, the server's in an online client; nothing here
    /// keeps time): the hour the look held when the clock was first read, plus
    /// the simulation time since, through the look's soft path. An hour set
    /// from elsewhere (a capture card, `--time-of-day` at start) or a
    /// simulation started again (its time going back) anchors it anew. Without
    /// a simulation (`--no-agents`) this is never called and the hour stays.
    fn follow_clock(&mut self, renderer: &Renderer) {
        if !renderer.look.options.lighting.smooth_time {
            return;
        }
        let now = renderer.look.time_of_day();
        let t = self.sim_time;
        let (h0, t0) = match self.clock {
            Some((h0, t0, last)) if last == now && t >= t0 => (h0, t0),
            _ => (now, t),
        };
        renderer.look.set_time_of_day_smooth(h0 + (t - t0) / 3600.0);
        self.clock = Some((h0, t0, renderer.look.time_of_day()));
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
        self.set_player_with_car_access(visual, player, cw, body_visible, dt, None);
    }

    pub fn set_player_with_car_access(&mut self, visual: &mut PlayerVisual, player: &eo_player::Player, cw: &eo_player::world::CollisionWorld, body_visible: bool, dt: f64, transfer: Option<&crate::car_access::Frame>) {
        self.extras.clear();
        self.test_rows.clear();
        if let Some(lib) = &self.lib {
            let mut test_drawn = None;
            if let Some(body) = &lib.test_player {
                let posed = match transfer {
                    Some(frame) => Some(visual.test.car_frame(body, frame)),
                    None => visual.test.frame(body, player, cw, body_visible, dt as f32),
                };
                if let Some((rows, instance)) = posed {
                    self.test_rows.extend_from_slice(rows);
                    self.extras.push(instance);
                    if player.mode == eo_player::Mode::OnFoot {
                        test_drawn = Some((instance.pos.map(f64::from), f64::from(body.height)));
                    }
                }
            }
            let door = transfer.map(|frame| (frame.vehicle, frame.door));
            visual.instances_with_door(lib, player, cw, body_visible && lib.test_player.is_none(), dt, &mut self.extras, door);
            if test_drawn.is_some() {
                // the demo avatar stands in for the pool body on a landmark deck too (eo-render deck_rooms.rs).
                eo_render::deck_rooms::set_player(test_drawn);
            }
        }
        self.mark_bores();
    }

    /// (captures) A sedan standing at `datum` facing `yaw` as this frame's only extra
    /// instance (marked in a tunnel's bore as the player's car is).
    pub fn show_car(&mut self, datum: [f64; 3], yaw: f32) {
        self.extras.clear();
        if let Some(lib) = &self.lib {
            let q = glam::Quat::from_rotation_y(yaw);
            if let Some(c) = eo_agents::feed::player_car(lib, PLAYER_CAR_BODY, datum, [q.x as f64, q.y as f64, q.z as f64, q.w as f64], 0.0, 0.0, 0.0, false, PLAYER_CAR_PAINT) {
                self.extras.push(c);
            }
        }
        self.mark_bores();
    }

    /// The player's body and cars in a tunnel's bore lit as the bore is (`--tunnels`; nothing without bores).
    pub fn mark_bores(&mut self) {
        for a in &mut self.extras {
            eo_agents::feed::mark_in_bore(a, self.structures.as_deref());
        }
    }

    pub fn capture_player(&mut self, feet: [f64; 3], yaw: f64, time: f32) -> Result<GpuAgent> {
        self.test_rows.clear();
        let lib = self.lib.as_ref().ok_or_else(|| anyhow::anyhow!("player scale reference requires the agent library"))?;
        if let Some(body) = &lib.test_player {
            let mut pose = eo_agents::test_characters::Pose::default();
            let clip = body.preview.unwrap_or_else(|| body.rig.idle());
            pose.sample(&body.rig, clip, time.clamp(0.0, body.rig.seconds(clip)), body.preview.is_some(), 1.0);
            self.test_rows = pose.rows;
            Ok(body.instance(feet.map(|v| v as f32), glam::Quat::from_rotation_y(yaw as f32)))
        } else {
            eo_agents::feed::player_body(lib, feet, yaw, 0.0, 0.0, f64::from(time))
                .ok_or_else(|| anyhow::anyhow!("no verified player avatar available for scale reference"))
        }
    }

    pub fn clear_test_pose(&mut self) {
        self.test_rows.clear();
    }

    /// The camera as the simulation's focus: (x, z, view yaw atan2(dx, dz)).
    pub fn focus_of(eye: Vec3, forward: Vec3) -> [f64; 3] {
        [eye.x as f64, eye.z as f64, (forward.x as f64).atan2(forward.z as f64)]
    }
}

/// The corridor layer on the drawn deck and streets (`corridor`), starting at `hour`.
fn build_corridors(paths: &eo_data::DataPaths, source: &eo_sim::native::NativeSource, gg: &eo_landmarks::GroundGrid, ground: &Ground, walkers: bool, deck_drawn: bool, hour: f64) -> Result<eo_sim::corridor::Corridors> {
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
    let surfaces = corridor::Surfaces { deck: deck_drawn.then_some(&deck), street: matches!(ground, Ground::Street(_)), paved: &paved, ground: &carpet };
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
    test: crate::test_characters::Playback,
}

/// The player's car body (src/vehicles/hero-sedan-glb.js `HERO_SEDAN_ASSET`: the sedan LOD0).
pub const PLAYER_CAR_BODY: &str = "sedan";
/// The player's car paint (the civilian set's `red`; the JS hero sedan takes its paint from the presentation draw).
pub const PLAYER_CAR_PAINT: &str = "#a8332b";

impl PlayerVisual {
    pub fn instances(&mut self, lib: &Library, player: &eo_player::Player, cw: &eo_player::world::CollisionWorld, body_visible: bool, dt: f64, out: &mut Vec<GpuAgent>) {
        self.instances_with_door(lib, player, cw, body_visible, dt, out, None);
    }

    fn instances_with_door(&mut self, lib: &Library, player: &eo_player::Player, cw: &eo_player::world::CollisionWorld, body_visible: bool, dt: f64, out: &mut Vec<GpuAgent>, door: Option<(usize, f32)>) {
        use eo_player::Mode;
        self.time += dt;
        let height = lib.meta.family_heights.first().copied().unwrap_or(1.75) as f64;
        self.phase = eo_agents::gait::advance_phase(self.phase, player.ground_speed, dt, height);
        let mut drawn = None;
        if player.mode == Mode::OnFoot && body_visible {
            let f = player.character.feet();
            if let Some(b) = eo_agents::feed::player_body(lib, [f.x, f.y, f.z], player.facing_yaw, player.ground_speed, self.phase, self.time) {
                out.push(b);
                drawn = Some(([f.x, f.y, f.z], height));
            }
        }
        // the body on a landmark deck: its contact occlusion and the room's light (eo-render deck_rooms.rs).
        eo_render::deck_rooms::set_player(drawn);
        self.spins.resize(player.vehicles.len(), 0.0);
        for (k, v) in player.vehicles.iter().enumerate() {
            let speed = v.forward_speed(cw);
            self.spins[k] += speed * dt / v.spec.wheel_radius.max(0.2);
            let (_, rot) = v.pose(cw);
            let datum = v.datum(cw);
            let steer = v.controller.wheels().first().map(|w| w.steering as f64).unwrap_or(0.0);
            let braking = v.controls.brake > 0.0 || v.controls.handbrake;
            if let Some(mut c) = eo_agents::feed::player_car(lib, PLAYER_CAR_BODY, [datum.x, datum.y, datum.z], [rot.x, rot.y, rot.z, rot.w], self.spins[k], steer, speed, braking, PLAYER_CAR_PAINT) {
                if let Some((i, fraction)) = door && i == k {
                    c.clip_phase = fraction;
                }
                out.push(c);
            }
        }
    }
}
