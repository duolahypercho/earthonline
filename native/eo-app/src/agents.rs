//! the live simulation (eo-sim's `AsyncSim`, on its own thread),
//! the agent feed (eo-agents, its own thread: grounding, bodies, paint,
//! clips) and the agent library (loaded on a thread at start), driven from
//! the render thread once a frame: post the frame's time and the focus (the
//! camera or the player), pick up the newest instances, upload them.
//!
//! `--no-agents` turns all of it off; `--agent-density N` multiplies the
//! default population (42 moving cars, People `medium`) N times in one region.
//!
//! `--crowd` (off by default; `--no-crowd` as before, the frame unchanged):
//! the street's population follows the place and the hour, and a camera cut
//! arrives at a street already lived in.
//!
//! * The people by place and hour (eo-sim `CrowdDensity`, consts.rs's crowd
//!   density model) in place of the People setting's fixed 40 bodies in 55 m:
//!   each footway holds people by its street class, the storeys fronting it,
//!   its share of walk-in shops and of bars and restaurants (the frontage
//!   survey: every building edge of the city file with its storeys and its
//!   street-level use, `frontage_edges`), at the day's footfall or the
//!   evening economy's, within 150 m of the focus (the renderer's crowd
//!   tiers draw to 240 m), at most 400 bodies.
//! * The simulation's hour is the drawn hour (`--time-of-day`, and each
//!   cut's): without the switch it starts at noon whatever is drawn.
//! * A cut (the focus jumping more than `CUT_JUMP_M` between two frames, or
//!   the drawn hour jumping more than `CUT_HOUR_H`: a teleport, a capture
//!   card, a menu's time change) rebuilds the simulation round the new focus
//!   at the new hour from the network source already loaded, off the render
//!   thread, and runs `CUT_WARMUP_STEPS` fixed steps (walkers mid-walk, cars
//!   in flow and queued at the lights) before it replaces the running one:
//!   the street fills in one step, already settled, instead of ramping in
//!   from the disc's edge (or, in a capture that does not step the clock,
//!   never: the cards drew the simulation of the bench's first path, metres
//!   to kilometres away). The same focus, hour and network give the same
//!   street. `settle` waits for a pending cut (the captures call it).
//! * Where the people walk: the footways stand on the drawn walks and stop
//!   and cross at the drawn kerbs (`drawn_sections`, from eo-street's plan),
//!   an unsignalled street is crossed only at a mapped crosswalk
//!   (`crossing_points`), and the paved plazas the landmarks lay are crossed
//!   by walks of their own (`plaza_paths`).
//! * The bodies: kept apart and out of the vehicles, and no twins in sight
//!   (the feed's `separate`, eo-agents `feed::separate`).

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
use std::sync::mpsc::Receiver;
use std::time::Instant;

/// The simulation's resident network: tiles within this of the focus (as `sim_bench`).
pub const SIM_RADIUS_M: f64 = 600.0;
/// Re-stream the network when the focus has moved this far.
const RESTREAM_M: f64 = 32.0;
/// Fixed steps run before the first frame so the population has filled in.
const WARMUP_STEPS: usize = 600;
pub const SEED_INT: u32 = 1;
/// `--crowd`: a focus jump (m, between two pumps) or a drawn-hour jump (h)
/// this large is a cut, and the steps a cut's new simulation runs before it
/// is shown (10 s: a walker 13 m on, a car through a signal cycle's start).
const CUT_JUMP_M: f64 = eo_sim::consts::CROWD_RADIUS_M;
const CUT_HOUR_H: f64 = 0.25;
const CUT_WARMUP_STEPS: usize = 600;

/// `--crowd` (main.rs sets it before the runtime starts; see the module's head).
pub static CROWD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn crowd_on() -> bool {
    CROWD.load(std::sync::atomic::Ordering::Relaxed)
}

/// The walk-in rule of the decal survey (streets.rs `cc0_survey`): a shop or
/// food `amenity`, or a retail / commercial `building`; not worship,
/// schools, services behind a lobby or parking. The evening economy's uses
/// count as `night` instead.
const NOT_SHOPS: &[&str] = &["place_of_worship", "school", "university", "kindergarten", "college", "parking", "fire_station", "police", "company", "social_facility", "clinic", "hospital", "doctors", "dentist", "toilets", "grave_yard", "prison", "townhall", "courthouse"];
const SHOP_BUILDINGS: &[&str] = &["retail", "commercial", "supermarket", "kiosk"];
const NIGHT_USES: &[&str] = &["bar", "pub", "restaurant", "nightclub", "biergarten", "theatre", "cinema", "arts_centre", "events_venue", "music_venue"];

/// `--crowd`: the walks of their own (eo-sim `NativeSource::with_paths`):
/// the city file's `pedestrian`, `footway` and `path` ways at grade (no
/// bridge, tunnel or layer) that are not a street's sidewalk or crossing (the
/// file lists those apart, `sidewalkWays` and `crossingWays`: the streets'
/// own footways stand for them): a plaza's, a promenade's, a park's paths.
pub fn walk_paths(paths: &eo_data::DataPaths, city: &eo_data::City) -> Vec<eo_sim::native::PathWay> {
    #[derive(serde::Deserialize)]
    struct Id {
        id: eo_data::RecordId,
    }
    #[derive(serde::Deserialize)]
    struct Lists {
        #[serde(rename = "sidewalkWays", default)]
        sidewalks: Vec<Id>,
        #[serde(rename = "crossingWays", default)]
        crossings: Vec<Id>,
    }
    let street_walks: std::collections::HashSet<String> = eo_data::read_maybe_gzip(&paths.sf("sf-city.json.gz"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Lists>(&b).ok())
        .map(|l| l.sidewalks.iter().chain(&l.crossings).map(|i| i.id.key()).collect())
        .unwrap_or_default();
    city.roads
        .iter()
        .filter(|r| matches!(r.highway.as_str(), "pedestrian" | "footway" | "path") && !r.bridge && !r.tunnel && r.layer == 0 && r.points64.len() >= 2 && !street_walks.contains(&r.id))
        .map(|r| eo_sim::native::PathWay { id: r.id.clone(), highway: r.highway.clone(), points: r.points64.clone() })
        .collect()
}

/// `--crowd`: the mapped crosswalks' middles (eo-sim
/// `NativeSource::with_crossings`): the city file's `crossingWays` (OSM
/// `footway=crossing`, marked, unmarked or signalled), on the scaled plan.
pub fn crossing_points(paths: &eo_data::DataPaths) -> Vec<[f64; 2]> {
    #[derive(serde::Deserialize)]
    struct Way {
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(serde::Deserialize)]
    struct Lists {
        #[serde(rename = "crossingWays", default)]
        crossings: Vec<Way>,
    }
    let gs = eo_data::groundscale::scale();
    eo_data::read_maybe_gzip(&paths.sf("sf-city.json.gz"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Lists>(&b).ok())
        .map(|l| {
            l.crossings
                .iter()
                .filter(|w| w.points.len() >= 4)
                .map(|w| {
                    let n = w.points.len() / 2;
                    let (sx, sz) = w.points.chunks_exact(2).fold((0.0, 0.0), |a, c| (a.0 + c[0], a.1 + c[1]));
                    [sx / n as f64 * gs, sz / n as f64 * gs]
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `--crowd`: the drawn street's cross-sections (eo-sim
/// `NativeSource::with_sections`): every straight piece of every drawn
/// carriageway's centreline with its half width and its walks (eo-street's
/// plan; a structure's leg into a junction is not drawn and is left out).
pub fn drawn_sections(sw: &eo_street::StreetWorld) -> Vec<eo_sim::native::DrawnSection> {
    sw.plan
        .entries
        .iter()
        .filter(|e| e.structure_leg.is_none() && e.half > 0.0)
        .flat_map(|e| e.points.windows(2).map(move |w| eo_sim::native::DrawnSection { a: w[0], b: w[1], half: e.half, walks: e.walks }))
        .collect()
}

/// `--crowd`: the paved plazas the landmarks lay (eo-landmarks `plaza::plan`
/// over sf-lots-parks, a footprint test from the drawn buildings): each
/// one's id and outer ring. None when the plazas do not load.
pub fn plaza_rings(root: &std::path::Path, sw: &eo_street::StreetWorld) -> Vec<(String, Vec<[f64; 2]>)> {
    let Ok((greens, water)) = eo_landmarks::plaza::load_areas(root) else { return vec![] };
    let k = &eo_landmarks::LandmarksJs::get().open_space.plaza;
    let plan = eo_landmarks::plaza::plan(k, &greens, &water, Some(&|x: f64, z: f64| sw.buildings.contains(x, z)));
    plan.plazas.iter().filter_map(|p| p.rings.first().map(|r| (p.id.clone(), r.clone()))).collect()
}

/// `--crowd`: the landmarks' walkable paving (`Structure::footway`: the
/// quay's apron along the seawall, the pier decks, the plazas' paving) as
/// triangles for the feed's `Decks`.
pub fn footway_decks(structures: &[eo_landmarks::set::Structure]) -> Vec<[[f32; 3]; 3]> {
    let mut out = vec![];
    for s in structures.iter().filter(|s| s.footway) {
        let p = &s.mesh.positions;
        let v = |i: u32| -> Option<[f32; 3]> { let k = 3 * i as usize; p.get(k..k + 3).map(|x| [x[0], x[1], x[2]]) };
        for t in s.mesh.indices.chunks_exact(3) {
            if let (Some(a), Some(b), Some(c)) = (v(t[0]), v(t[1]), v(t[2])) {
                out.push([a, b, c]);
            }
        }
    }
    out
}

/// `--crowd`: the plazas' walks (eo-sim `plaza_walks`): each crossed by
/// straight walks between points of its outline and walked round.
pub fn plaza_paths(rings: &[(String, Vec<[f64; 2]>)]) -> Vec<eo_sim::native::PathWay> {
    rings.iter().flat_map(|(id, r)| eo_sim::native::plaza_walks(id, r)).collect()
}

/// `--crowd`: every building edge of the city file as a front the footways
/// are measured against (eo-sim `NativeSource::with_frontage`): its storeys
/// (the `levels` tag, else the resolved height over the kit's storey) and
/// its street-level use.
pub fn frontage_edges(city: &eo_data::City) -> Vec<eo_sim::native::FrontEdge> {
    let mut out = Vec::with_capacity(city.buildings.len() * 6);
    for b in &city.buildings {
        let n = b.ring.len();
        if n < 3 {
            continue;
        }
        let storeys = b.tags.levels.filter(|l| *l >= 1.0).unwrap_or(b.height as f64 / eo_kit::classify::STOREY_METRES).max(1.0) as f32;
        let am = b.tags.amenity.as_str();
        let night = NIGHT_USES.contains(&am);
        let shops = !night && ((!am.is_empty() && !NOT_SHOPS.contains(&am)) || SHOP_BUILDINGS.contains(&b.tags.building.as_str()));
        for i in 0..n {
            let (p, q) = (b.ring[i], b.ring[(i + 1) % n]);
            out.push(eo_sim::native::FrontEdge { a: [p[0] as f64, p[1] as f64], b: [q[0] as f64, q[1] as f64], storeys, shops, night });
        }
    }
    out
}

/// (`--car-traffic`; off by default, the cards as before) The capture cards'
/// streets as a player arriving there finds them (bench `--light-shots`):
/// before each card is drawn the simulation is built again round its eye
/// (`AgentRuntime::warm_traffic`, the game's own start: the network round
/// the focus, its cars and walkers placed and run `WARMUP_STEPS` fixed
/// steps), instead of the one built round the first view at start, which
/// never moves while the cards are drawn and leaves their streets empty.
static WARM_TRAFFIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// While a warmed card is drawn, simulated agents whose body comes within
/// this of its eye are not drawn (no bus or walker filling the foreground:
/// the card's subject stays in view, as a crew holds traffic out of a shot).
pub const WARM_CLEAR_M: f32 = 8.0;
/// How long a card waits for its simulation and the feed's first build.
const WARM_WAIT_S: f64 = 120.0;

/// `--car-traffic` / `--no-car-traffic`.
pub fn set_warm_traffic(on: bool) {
    WARM_TRAFFIC.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn warm_traffic_on() -> bool {
    WARM_TRAFFIC.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
pub struct AgentOpts {
    pub on: bool,
    pub density: u32,
    /// `--car-detail` (0 as before): the library's far bodies and the renderer's vehicle tiers.
    pub car_detail: u8,
    /// `--crowd` (false as before): the population by place and hour, cuts settled.
    pub crowd: bool,
}

impl Default for AgentOpts {
    fn default() -> Self {
        Self { on: true, density: 1, car_detail: 0, crowd: false }
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
    pub sim: Option<AsyncSim>,
    feed: Option<FeedThread>,
    ground: Ground,
    /// the grade-separated roads beside the ground (`--structures`), handed to the feed.
    structures: Option<Arc<eo_structures::StructureWorld>>,
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
    /// `--smooth-time`: the drawn hour as anchored to the simulation's clock
    /// (the look's hour then, the simulation time then, the hour last set).
    clock: Option<(f64, f64, f64)>,
    /// groundscale (captures only, empty otherwise): simulated agents whose bounding
    /// radius comes within these (point, horizontal margin) of the point are not drawn
    /// (a traffic car driven through the player's car or the lens; the simulation does
    /// not see the player's car).
    pub hide_near: Vec<([f32; 3], f32)>,
    /// `--crowd`: the city file (the frontage survey's buildings), the
    /// network source once loaded (a cut builds from it), the hour the
    /// simulation starts at, the last pump's focus and drawn hour, the cut
    /// being built (its focus and hour), and the feed post the cut's first
    /// frame went out as (cleared once its instances are back).
    city: Option<Arc<eo_data::City>>,
    source: Option<Arc<eo_sim::native::NativeSource>>,
    start_hour: f64,
    last_focus: Option<([f64; 3], f64)>,
    cut_rx: Option<(Receiver<Result<SimWorld, String>>, [f64; 3], f64)>,
    cut_post: Option<u64>,
    /// `--crowd`: cuts made (the capture JSON reports them).
    pub cuts: u64,
    /// `--crowd`: the paved plazas (set by the simulation's builder, handed to the feed).
    plazas: Arc<std::sync::Mutex<Option<Arc<eo_agents::feed::Plazas>>>>,
    /// `--crowd`: the landmarks' walkable paving (handed to the feed).
    decks: Option<Arc<eo_agents::feed::Decks>>,
}

/// One region around `focus` at `density` times the default population;
/// with `crowd` (`--crowd`) the people by place and `hour` instead of the
/// People setting's disc, the clock starting at `hour` (else noon, as before).
fn region_config(density: u32, crowd: Option<f64>) -> RegionConfig {
    let mut cfg = RegionConfig::game_default(SEED_INT);
    let n = density.max(1) as usize;
    cfg.vehicle_pool *= n;
    if let People::Bubble(b) = &mut cfg.people {
        b.max_bodies *= n;
    }
    if let Some(hour) = crowd {
        cfg.hour = hour.rem_euclid(24.0);
        cfg.crowd = Some(eo_sim::consts::CrowdDensity::standard(n as f64));
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
        let opts = AgentOpts { on: args.agents, density: args.agent_density, car_detail: args.car_detail, crowd: crowd_on() };
        let ground = match &world.street {
            Some(s) => Ground::Street(s.world.clone()),
            None => Ground::Elevation(Arc::new(world.elevation.clone())),
        };
        let mut rt = AgentRuntime {
            opts,
            lib: None,
            lib_rx: None,
            sim_rx: None,
            sim: None,
            feed: None,
            ground,
            structures: world.structures.clone(),
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
            clock: None, // smooth time
            // `--crowd`
            city: opts.crowd.then(|| world.city.clone()),
            source: None,
            start_hour: args.time_of_day,
            last_focus: None,
            cut_rx: None,
            cut_post: None,
            cuts: 0,
            plazas: Default::default(),
            decks: (opts.crowd && !world.footway_decks.is_empty()).then(|| Arc::new(eo_agents::feed::Decks::new(world.footway_decks.as_ref().clone()))),
        };
        if !opts.on {
            return rt;
        }
        let repo = world.paths.repo_root.clone();
        let cache = crate::cache_dir(args);
        // the Rocketbox people unless `--no-rocketbox` (or no copy at EO_PEOPLE_ROOT): then none.
        let people = if args.rocketbox { eo_agents::library::PeopleSource::from_env() } else { eo_agents::library::PeopleSource::Off };
        // (`--car-detail`) each body's far slot its own simplified body instead of three boxes.
        // (`--car-materials`) each vehicle material's role and the plates' own coordinates;
        // (`--car-contact`) the bodies' occluder boxes, cooked with them (nothing of the
        // roles draws without `--car-materials`'s shader code); (`--people-motion`) the
        // people's heads held steady in the bake.
        let cook = eo_agents::library::CookOptions { far_body: opts.car_detail > 0, car_materials: args.look.car_materials > 0 || args.look.car_contact > 0, steady_heads: eo_agents::motion::steady_heads() };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-load".into())
            .spawn(move || {
                // (`--people-contact`) the people's contact anchors, made at load (never cached);
                // (`--people-capsules`) their body capsules in their place.
                let lib = Library::load_opts(&repo, cache.as_deref(), &people, cook).map(|mut l| {
                    if eo_render::agents::people_capsules() {
                        eo_agents::people::add_capsule_anchors(&mut l);
                    } else if eo_render::agents::people_contact() {
                        eo_agents::people::add_contact_anchors(&mut l);
                    }
                    l
                });
                let _ = tx.send(lib.map_err(|e| format!("{e:#}")));
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
        // `--crowd`: the start hour, and the frontage survey (none without the switch).
        self.cut_rx = None;
        self.cut_post = None;
        self.last_focus = None;
        let crowd = self.opts.crowd.then_some(self.start_hour);
        let city = self.city.clone();
        let paths = world.paths.clone();
        // `--crowd`: the drawn street (the walks the footways stand on, the plazas' paving).
        let street = if self.opts.crowd { world.street.as_ref().map(|s| s.world.clone()) } else { None };
        let plazas_out = self.plazas.clone();
        // The structure level of each way (`--structures`; empty otherwise, which leaves the sim as it was).
        let levels = self.structures.as_ref().map(|s| s.way_levels().clone()).unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-sim-build".into())
            .spawn(move || {
                let r = (|| -> Result<(SimWorld, Arc<eo_sim::native::NativeSource>)> {
                    let mut source = eo_sim::native::load(&paths)?.with_levels(levels);
                    if let Some(c) = &city {
                        // `--crowd`: the fronts, the walks of their own and the plazas', the
                        // drawn walks and kerbs, the mapped crosswalks.
                        let mut walks = walk_paths(&paths, c);
                        let mut sections = vec![];
                        if let Some(sw) = &street {
                            let rings = plaza_rings(&paths.repo_root, sw);
                            walks.extend(plaza_paths(&rings));
                            *plazas_out.lock().unwrap() = Some(Arc::new(eo_agents::feed::Plazas::new(rings.into_iter().map(|(_, r)| r).collect())));
                            sections = drawn_sections(sw);
                        }
                        source = source.with_frontage(frontage_edges(c)).with_paths(walks).with_sections(sections).with_crossings(crossing_points(&paths));
                    }
                    let source = Arc::new(source);
                    let w = build_sim(&source, focus, density, crowd, WARMUP_STEPS);
                    Ok((w, source))
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
                        if self.opts.crowd {
                            self.source = Some(src.clone()); // `--crowd`: a cut builds from it
                        }
                        self.sim = Some(start_async(w, src));
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
                    Ok(mut f) => {
                        f.separate = self.opts.crowd; // `--crowd`: the bodies kept apart
                        f.plazas = self.plazas.lock().unwrap().clone(); // `--crowd`: on the plazas' paving
                        f.decks = self.decks.clone(); // `--crowd`: on the landmarks' paving
                        self.feed = Some(FeedThread::start(f));
                    }
                    Err(e) => self.errors.push(format!("{e:#}")),
                }
            }
        }
        if self.opts.crowd && !self.remote_mode {
            self.cut_check(focus, renderer.look.time_of_day()); // `--crowd`
        }
        let gpu = renderer.agents.get();
        if let Some(g) = gpu {
            g.set_enabled(self.opts.on);
        }
        let mut clock_read = false; // smooth time
        if let Some(sim) = &mut self.sim {
            sim.post(dt, &[(0, Focus { x: focus[0], z: focus[1], yaw: focus[2] })]);
            let fr = sim.latest();
            self.sim_time = fr.time;
            clock_read = true;
            if fr.steps != self.last_steps {
                self.last_steps = fr.steps;
                if let Some(feed) = &self.feed {
                    let seq = feed.post(fr.clone());
                    if self.cut_post == Some(0) {
                        self.cut_post = Some(seq); // `--crowd`: the cut's first frame
                    }
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
            let (out, ms) = feed.take_seq();
            if let Some((o, seq)) = out {
                self.last = Some(o);
                if self.cut_post.is_some_and(|p| p > 0 && seq >= p) {
                    self.cut_post = None; // `--crowd`: the cut's street is what is drawn
                }
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

    /// `--crowd`: start a cut when the focus or the drawn hour jumps; swap a
    /// finished cut in (or build it again when the focus moved on meanwhile).
    fn cut_check(&mut self, focus: [f64; 3], hour: f64) {
        let jumped = |a: &[f64; 3], ha: f64, b: &[f64; 3], hb: f64| {
            let dh = (ha - hb).rem_euclid(24.0);
            (a[0] - b[0]).hypot(a[1] - b[1]) > CUT_JUMP_M || dh.min(24.0 - dh) > CUT_HOUR_H
        };
        if let Some((rx, at, h)) = &self.cut_rx {
            match rx.try_recv() {
                Ok(Ok(w)) => {
                    let stale = jumped(at, *h, &focus, hour);
                    self.cut_rx = None;
                    if !stale {
                        if let Some(src) = self.source.clone() {
                            self.sim = Some(start_async(w, src));
                            self.last_steps = 0;
                            self.cut_post = Some(0); // the next post is the cut's first frame
                            self.cuts += 1;
                        }
                    } else {
                        self.start_cut(focus, hour);
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("agents: cut failed: {e}");
                    self.errors.push(e);
                    self.cut_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.cut_rx = None,
            }
        } else if self.sim.is_some() {
            if let Some((f, h)) = self.last_focus {
                if jumped(&f, h, &focus, hour) {
                    self.start_cut(focus, hour);
                }
            }
        }
        self.last_focus = Some((focus, hour));
    }

    /// Build the simulation round `focus` at `hour` on a thread (`--crowd`).
    fn start_cut(&mut self, focus: [f64; 3], hour: f64) {
        let Some(src) = self.source.clone() else { return };
        let density = self.opts.density;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-sim-cut".into())
            .spawn(move || {
                let _ = tx.send(Ok(build_sim(&src, focus, density, Some(hour), CUT_WARMUP_STEPS)));
            })
            .expect("spawn simulation cut");
        self.cut_rx = Some((rx, focus, hour));
    }

    /// Whether a cut is being built or its street is not drawn yet (`--crowd`; always false without it).
    pub fn cut_pending(&self) -> bool {
        self.cut_rx.is_some() || self.cut_post.is_some()
    }

    /// Pump (time held) until a pending cut's street is drawn, at most
    /// `timeout_s` (`--crowd`; returns at once without it or without a cut).
    pub fn settle(&mut self, ctx: &GpuContext, renderer: &Renderer, focus: [f64; 3], timeout_s: f64) {
        if !self.opts.crowd {
            return;
        }
        let t = Instant::now();
        self.pump(ctx, renderer, focus, 0.0);
        while self.cut_pending() && t.elapsed().as_secs_f64() < timeout_s {
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.pump(ctx, renderer, focus, 0.0);
        }
        if self.cut_pending() {
            eprintln!("agents: cut still pending after {timeout_s:.0} s");
        }
    }

    /// `--crowd` (captures): the simulated people and vehicles drawn now,
    /// all and within 50 / 150 m of `eye`, and the cuts made so far.
    pub fn census_json(&self, eye: Vec3) -> serde_json::Value {
        let lib = self.lib.as_deref();
        let person = |a: &GpuAgent| lib.and_then(|l| l.models.get(a.model as usize)).is_some_and(|m| m.class == eo_agents::CLASS_PERSON);
        let list = self.last.as_deref().map(|v| v.as_slice()).unwrap_or(&[]);
        let near = |a: &GpuAgent, r: f32| Vec3::from(a.pos).distance(eye) < r;
        let count = |p: bool, r: f32| list.iter().filter(|a| person(a) == p && near(a, r)).count();
        serde_json::json!({
            "people": count(true, f32::INFINITY), "people_50m": count(true, 50.0), "people_150m": count(true, 150.0),
            "vehicles": count(false, f32::INFINITY), "vehicles_50m": count(false, 50.0), "vehicles_150m": count(false, 150.0),
            "parked": self.parked.len(), "cuts": self.cuts,
        })
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
        self.extras.clear();
        if let Some(lib) = &self.lib {
            visual.instances(lib, player, cw, body_visible, dt, &mut self.extras);
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

    /// (`--car-traffic`, captures) The simulation built again round `eye`
    /// looking along `forward` (`restart_sim`: deterministic, the same
    /// population every run) and waited on until the feed has drawn it; the
    /// simulated agents within `WARM_CLEAR_M` of the eye hidden until
    /// `hide_near` is cleared. Returns the vehicles and people it drew, or
    /// None (nothing warmed: no simulation here, or not in time).
    pub fn warm_traffic(&mut self, ctx: &GpuContext, renderer: &Renderer, world: &World, eye: Vec3, forward: Vec3) -> Option<(usize, usize)> {
        if self.remote_mode || !self.opts.on {
            return None;
        }
        let focus = Self::focus_of(eye, forward);
        if self.opts.crowd {
            // `--crowd`: the card's street at the card's hour (the restart starts the
            // clock at `start_hour`; it was the run's start hour for every card).
            self.start_hour = renderer.look.time_of_day();
        }
        self.restart_sim(world, focus, self.opts.density);
        self.hide_near = vec![(eye.to_array(), WARM_CLEAR_M)];
        let t = Instant::now();
        while t.elapsed().as_secs_f64() < WARM_WAIT_S {
            self.pump(ctx, renderer, focus, 0.0);
            if let (Some(_), Some(last), Some(lib)) = (&self.sim, &self.last, &self.lib) {
                let people = last.iter().filter(|a| lib.models.get(a.model as usize).is_some_and(|m| m.class == eo_agents::CLASS_PERSON)).count();
                return Some((last.len() - people, people));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        eprintln!("agents: the warmed traffic did not come in {WARM_WAIT_S:.0} s");
        None
    }

    /// The camera as the simulation's focus: (x, z, view yaw atan2(dx, dz)).
    pub fn focus_of(eye: Vec3, forward: Vec3) -> [f64; 3] {
        [eye.x as f64, eye.z as f64, (forward.x as f64).atan2(forward.z as f64)]
    }
}

/// A simulation round `focus`: the network within `SIM_RADIUS_M` from the
/// source, one region (`region_config`), `steps` fixed steps run.
fn build_sim(source: &eo_sim::native::NativeSource, focus: [f64; 3], density: u32, crowd: Option<f64>, steps: usize) -> SimWorld {
    let tiles = eo_sim::native::tiles_around(focus[0], focus[1], SIM_RADIUS_M);
    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&tiles));
    let mut w = SimWorld::new(net);
    let f = Focus { x: focus[0], z: focus[1], yaw: focus[2] };
    let r = Region::new(&w.net, 0, region_config(density, crowd), f);
    w.regions.push(r);
    for _ in 0..steps {
        w.step();
    }
    if let (Some(hour), Some(r)) = (crowd, w.regions.first()) {
        if let Some(d) = r.cfg.crowd {
            let c = r.crowd_census(&w.net, d, hour);
            eprintln!(
                "crowd: ({:.0}, {:.0}) at {hour:.2} h: {} footways, {:.0} m within {:.0} m; shops {:.2}, nightlife {:.2}, storeys {:.1}, class {:.2} (length-weighted); {:.0} people expected, {} walkers and {} cars after {steps} steps",
                focus[0], focus[1], c.footways, c.length_m, d.radius_m, c.shops, c.night, c.storeys, c.class_weight, c.people, r.crowd.walkers.len(), r.fleet.len()
            );
        }
    }
    w
}

/// The simulation on its own thread, streaming the network round its focus.
fn start_async(w: SimWorld, src: Arc<eo_sim::native::NativeSource>) -> AsyncSim {
    let mut last = [f64::NAN; 2];
    AsyncSim::start_with(w, move |w| {
        let f = w.regions.first().map(|r| [r.focus.x, r.focus.z]).unwrap_or([0.0; 2]);
        if !((f[0] - last[0]).hypot(f[1] - last[1]) < RESTREAM_M) {
            last = f;
            eo_sim::native::stream(w, &src, SIM_RADIUS_M);
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `--crowd`'s population at the capture cards' places and hours (a
    /// diagnostic: `cargo test --release -p eo-app crowd_at_the_cards -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn crowd_at_the_cards() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = eo_data::City::load(&paths).unwrap();
        let walks = walk_paths(&paths, &city);
        eprintln!("walks of their own: {} ({} pedestrian)", walks.len(), walks.iter().filter(|w| w.highway == "pedestrian").count());
        let source = eo_sim::native::load(&paths).unwrap().with_frontage(frontage_edges(&city)).with_paths(walks);
        for (name, x, z, hour) in [
            ("alley", 1712.0, -1439.0, 12.0),
            ("plaza", 2606.0, -2366.0, 12.0),
            ("castro-night", -1710.0, 2353.0, 21.0),
            ("mission-wet", 100.0, 3520.0, 22.0),
            ("downtown-golden", 1883.0, -1799.0, 18.5),
            ("viaduct-golden", 135.0, 1136.0, 18.5),
            ("embarcadero-blue", 3087.0, -2068.0, 19.25),
        ] {
            let t = std::time::Instant::now();
            let _ = build_sim(&source, [x, z, 0.0], 1, Some(hour), 60);
            eprintln!("  {name}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        }
    }

    /// The frontage survey reads every building edge, its storeys and its use.
    #[test]
    fn frontage_edges_carry_use_and_storeys() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = eo_data::City::load(&paths).unwrap();
        let edges = frontage_edges(&city);
        assert!(edges.len() > city.buildings.len() * 3, "every ring's edges");
        assert!(edges.iter().any(|e| e.night) && edges.iter().any(|e| e.shops), "both uses found");
        assert!(edges.iter().all(|e| e.storeys >= 1.0 && !(e.shops && e.night)));
    }
}
