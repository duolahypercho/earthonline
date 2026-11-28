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

mod corridor;

use crate::{Args, World};
use anyhow::Result;
use eo_agents::GpuAgent;
use eo_agents::catalog::AgentsJs;
use eo_agents::feed::{FeedThread, Feeder, Ground, car_half_base};
use eo_agents::library::Library;
use eo_player::traffic::{TrafficBodies, TrafficChassis, TrafficPose};
use eo_sim::consts::{PRESENTED_LENGTH, PRESENTED_WIDTH};
use eo_render::{GpuContext, Renderer};
use eo_sim::region::{Focus, People, Region, RegionConfig};
use eo_sim::vehicles::TrafficObstacle;
use eo_sim::{AsyncSim, World as SimWorld};
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
pub const SEED_INT: u32 = eo_sim::vehicle_catalogue::RENDERED_SEED;
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

/// `--people-groups` (with `--crowd`; main.rs sets it): social groups keep
/// together (eo-sim `RegionConfig::groups`, `Crowd::keep_parties`).
pub static PEOPLE_GROUPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--people-crosswalks` (with `--crowd`; main.rs sets it): the walkers cross
/// on the zebras the street draws (eo-sim `NativeSource::with_zebras`).
pub static PEOPLE_CROSSWALKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--people-near` (captures; main.rs sets it): a warmed card hides the
/// simulated people only within `PEOPLE_NEAR_CLEAR_M` of its eye, not
/// `WARM_CLEAR_M`: a crew holds traffic out of a shot, not the people on the
/// walk round the camera.
pub static PEOPLE_NEAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// (`--people-near`) the people hidden round a warmed card's eye: a person
/// nearer than this (and their bounding radius) would fill a street lens's
/// frame (a body 1.8 m tall is the whole height of a 30 degree frame at 3.4 m).
pub const PEOPLE_NEAR_CLEAR_M: f32 = 4.0;

/// `--people-windows` (with `--crowd`; main.rs sets it): a window-shopping
/// stop only in front of a shop's window (eo-sim `RegionConfig::windows`).
pub static PEOPLE_WINDOWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--people-stagger` (with `--crowd` and `--people-groups`; main.rs sets it):
/// a walking group's line turned from square by its own angle (eo-sim
/// `RegionConfig::stagger`).
pub static PEOPLE_STAGGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--people-evening` (with `--crowd`; main.rs sets it): the walk-in shops'
/// walkers by a neighbourhood commercial street's day, which holds into the
/// evening (eo-sim `shop_footfall`, `CrowdDensity::evening`).
pub static PEOPLE_EVENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn people_groups() -> bool {
    PEOPLE_GROUPS.load(std::sync::atomic::Ordering::Relaxed)
}

fn people_crosswalks() -> bool {
    PEOPLE_CROSSWALKS.load(std::sync::atomic::Ordering::Relaxed)
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

/// `--people-crosswalks`: the zebras the street draws (eo-sim
/// `NativeSource::with_zebras`): at each junction that earns paint, each
/// approach whose crossing is painted and fits its run (eo-street mesh.rs
/// `emit_approach_paint`'s rule), the middle of its band on the approach's
/// centreline (the band from the approach's trim plus the crosswalk
/// clearance, the junction's band depth deep). A structure's leg carries none.
pub fn drawn_zebras(sw: &eo_street::StreetWorld) -> Vec<[f64; 2]> {
    let o = &sw.o;
    let mut out = vec![];
    for node in sw.plan.nodes.iter().filter(|n| n.earns_paint) {
        for app in &node.approaches {
            let e = &sw.plan.entries[app.entry];
            if e.structure_leg.is_some() || !app.crossing_painted {
                continue;
            }
            let available = app.run_length - if app.at_start { e.trim_end } else { e.trim_start };
            let band0 = app.trim + o.crosswalk_clearance;
            let band1 = band0 + node.band_depth;
            if band1 + 0.4 > available {
                continue;
            }
            let d = 0.5 * (band0 + band1);
            let f = e.frame(if app.at_start { d } else { e.length - d }, false);
            out.push([f.x, f.z]);
        }
    }
    out
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

/// Compare traffic using the paved cross-sections or the mapped road profile.
pub fn traffic_street_plan_on() -> bool {
    std::env::args().filter(|arg| matches!(arg.as_str(), "--traffic-street-plan" | "--no-traffic-street-plan"))
        .last().is_some_and(|arg| arg == "--traffic-street-plan")
}

pub fn traffic_passing_on() -> bool {
    !std::env::args().any(|arg| arg == "--no-traffic-passing")
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

pub(super) fn bridge_walk(access: Option<&eo_landmarks::access::AccessSet>, elevation: &eo_data::ElevationGrid) -> Arc<eo_agents::feed::Decks> {
    let drawn = match access.and_then(|set| set.deck("golden-gate-east-sidewalk")) {
        Some(deck) => eo_agents::feed::Decks::from_world_triangles(deck.walk.iter().copied()),
        None => {
            // The base bridge still draws the sidewalk with landmark access off.
            let deck = eo_net::game::base_bridge_walk(elevation);
            eo_agents::feed::Decks::from_world_triangles(deck.walk)
        }
    };
    Arc::new(drawn)
}

/// Coastal walk surfaces are part of the base world. Expanded crowds also
/// visit the optional plaza and pier routes.
pub(crate) fn walker_decks(world: &World, crowd: bool) -> Option<Arc<eo_agents::feed::Decks>> {
    let mut floors = if crowd { world.footway_decks.as_ref().clone() } else { Vec::new() };
    for promenade in &world.promenades {
        floors.extend_from_slice(&promenade.walk);
    }
    (!floors.is_empty()).then(|| Arc::new(eo_agents::feed::Decks::new(floors)))
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

/// (`--car-density`; off by default, the JS pool as before) The traffic by
/// street class and hour (eo-sim `TrafficDensity`): each lane round the focus
/// carries its class's density at the drawn hour's share of the busiest
/// (`lane_density_per_km`, `hour_footfall`), the fleet up to
/// `DENSE_VEHICLE_POOL` cars, the local recycler keeping the near streets'
/// share; the capture cards take it at their own hour.
static CAR_DENSITY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--car-density` / `--no-car-density`.
pub fn set_car_density(on: bool) {
    CAR_DENSITY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn car_density_on() -> bool {
    CAR_DENSITY.load(std::sync::atomic::Ordering::Relaxed)
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

struct TrafficModels {
    catalog: AgentsJs,
    chassis: Vec<Option<TrafficChassis>>,
}

impl TrafficModels {
    fn load(lib: &Library) -> Result<Self> {
        let scale = eo_data::groundscale::vehicles() as f32;
        let mut chassis = vec![None; lib.models.len()];
        for &id in lib.meta.vehicle_models.values() {
            let Some(geometry) = lib.vehicle_chassis_geometry(id, scale) else { continue };
            chassis[id as usize] = Some(TrafficChassis::Mesh {
                min: glam::Vec3::from_array(geometry.min_xyz_m),
                max: glam::Vec3::from_array(geometry.max_xyz_m),
                wheels: geometry.wheels_xz_m,
                wheel_radius: geometry.wheel_radius_m,
            });
        }
        Ok(Self { catalog: AgentsJs::load()?, chassis })
    }

    fn parked_target(&self, lib: &Library, a: &GpuAgent, props: Option<&crate::props::PropsRuntime>) -> Option<eo_player::traffic_contact::TrafficTarget> {
        let name = lib.meta.model_names.get(a.model as usize)?;
        // Parked instances and the cached meshes use the same vehicle scale.
        let chassis = match self.chassis.get(a.model as usize).copied().flatten() {
            Some(mesh) => mesh,
            None => {
                let spec = props?.parked_spec(name)?;
                TrafficChassis::Class { width: spec.width as f32 * a.scale, length: spec.length as f32 * a.scale }
            }
        };
        let fwd = glam::Quat::from_array(a.rot) * glam::Vec3::Z;
        Some(eo_player::traffic_contact::TrafficTarget {
            model: eo_player::traffic_contact::TrafficModel::from_body(name),
            parked: true, speed: 0.0,
            pose: TrafficPose { id: parked_id(a), datum: a.pos, yaw: fwd.x.atan2(fwd.z), pitch: fwd.y.asin(), chassis },
        })
    }

    fn select(&self, lib: &Library, entity: u32, kind: u16) -> Option<(TrafficChassis, f64, eo_player::traffic_contact::TrafficModel)> {
        let (body, model) = lib.traffic_model(&self.catalog, SEED_INT, entity, kind)?;
        let chassis = match self.chassis.get(model as usize).copied().flatten() {
            Some(chassis) => chassis,
            None => {
                let scale = eo_data::groundscale::vehicles() as f32;
                TrafficChassis::Class {
                    length: *PRESENTED_LENGTH.get(kind as usize)? * scale,
                    width: *PRESENTED_WIDTH.get(kind as usize)? * scale,
                }
            }
        };
        let actual = lib.meta.model_names.get(model as usize).map_or(body, String::as_str);
        Some((chassis, car_half_base(&lib.models[model as usize]), eo_player::traffic_contact::TrafficModel::from_body(actual)))
    }
}

pub struct AgentRuntime {
    pub opts: AgentOpts,
    traffic_obstacle: Option<TrafficObstacle>,
    on_foot_player: Option<TrafficObstacle>,
    pub lib: Option<Arc<Library>>,
    traffic_models: Option<TrafficModels>,
    lib_rx: Option<Receiver<Result<(Library, TrafficModels, Option<String>), String>>>,
    sim_rx: Option<Receiver<Result<(SimWorld, Arc<eo_sim::native::NativeSource>), String>>>,
    /// The corridor layer's hour (written from the look's time of day each pump).
    corridor_hour: Option<Arc<AtomicU64>>,
    /// The look's time of day at the last pump (the hour a rebuilt corridor layer starts at).
    last_hour: f64,
    /// The Golden Gate deck's ground grid (the drawn terrain under it), for the corridor layer.
    gg_ground: Option<Arc<eo_landmarks::GroundGrid>>,
    pub sim: Option<AsyncSim>,
    feed: Option<FeedThread>,
    feed_failed: bool,
    ground: Ground,
    /// the grade-separated roads beside the ground (`--structures`), handed to the feed.
    structures: Option<Arc<eo_structures::StructureWorld>>,
    physical_traffic: eo_agents::feed::TrafficPoses,
    walker_slots: Vec<eo_agents::feed::WalkerSlot>,
    physical_walkers: std::collections::HashMap<u64, eo_player::people_contact::WalkerPose>,
    posed_agents: Vec<GpuAgent>,
    reset_walkers: bool,
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
    pub remote_traffic: eo_agents::feed::TrafficPoses,
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
    /// `--crowd`: the city file (the frontage survey's buildings), the
    /// network source once loaded (a cut builds from it), the hour the
    /// simulation starts at, the last pump's focus and drawn hour, the cut
    /// being built (its focus and hour), and the feed post the cut's first
    /// frame went out as (cleared once its instances are back).
    city: Option<Arc<eo_data::City>>,
    source: Option<Arc<eo_sim::native::NativeSource>>,
    start_hour: f64,
    /// (`--car-density`) the hour the traffic's density is taken at (the start's; a warmed card's own)
    traffic_hour: f64,
    last_focus: Option<([f64; 3], f64)>,
    cut_rx: Option<(Receiver<Result<SimWorld, String>>, [f64; 3], f64)>,
    cut_post: Option<u64>,
    /// `--crowd`: cuts made (the capture JSON reports them).
    pub cuts: u64,
    /// `--crowd`: the paved plazas (set by the simulation's builder, handed to the feed).
    plazas: Arc<std::sync::Mutex<Option<Arc<eo_agents::feed::Plazas>>>>,
    /// `--crowd`: the landmarks' walkable paving (handed to the feed).
    decks: Option<Arc<eo_agents::feed::Decks>>,
    bridge_walk: Option<Arc<eo_agents::feed::Decks>>,
}

/// One region around `focus` at `density` times the default population;
/// with `crowd` (`--crowd`) the people by place and `hour` instead of the
/// People setting's disc, the clock starting at `hour` (else noon, as before).
fn region_config(density: u32, crowd: Option<f64>, traffic: Option<f64>) -> RegionConfig {
    let mut cfg = RegionConfig::game_default(SEED_INT);
    let n = density.max(1) as usize;
    if let Some(hour) = traffic {
        // (`--car-density`) the traffic by street class at the hour
        cfg.vehicle_pool = eo_sim::consts::DENSE_VEHICLE_POOL;
        cfg.traffic = Some(eo_sim::region::TrafficDensity::standard(hour.rem_euclid(24.0)));
    }
    cfg.vehicle_pool *= n;
    if let People::Bubble(b) = &mut cfg.people {
        b.max_bodies *= n;
    }
    if let Some(hour) = crowd {
        cfg.hour = hour.rem_euclid(24.0);
        cfg.crowd = Some(eo_sim::consts::CrowdDensity { evening: PEOPLE_EVENING.load(std::sync::atomic::Ordering::Relaxed), ..eo_sim::consts::CrowdDensity::standard(n as f64) }); // (`--people-evening`)
        cfg.groups = people_groups(); // (`--people-groups`) the groups keep together
        cfg.windows = PEOPLE_WINDOWS.load(std::sync::atomic::Ordering::Relaxed); // (`--people-windows`) browsing at shop windows only
        cfg.stagger = PEOPLE_STAGGER.load(std::sync::atomic::Ordering::Relaxed); // (`--people-stagger`) the groups' lines turned
    }
    cfg
}

// A parked slot's authored coordinates survive ring sorting and tile recooks.
// The upper bits separate parking slots, regional traffic and corridor traffic.
fn parked_id(instance: &GpuAgent) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    instance.pos.map(f32::to_bits).hash(&mut hash);
    (1 << 62) | (hash.finish() & ((1 << 62) - 1))
}

fn apply_walker_poses(agents: &mut [GpuAgent], slots: &[eo_agents::feed::WalkerSlot],
    poses: &std::collections::HashMap<u64, eo_player::people_contact::WalkerPose>, library: Option<&Library>) {
    for slot in slots {
        if let Some(p) = poses.get(&slot.id) {
            let a = &mut agents[slot.index];
            if a.model != p.model || glam::Vec3::from_array(slot.feet).distance_squared(glam::Vec3::from_array(p.route_feet)) >= 36.0 { continue; }
            a.pos = p.feet;
            a.rot = p.rotation;
            a.speed = p.walking_speed;
            a.anim = library.map_or(p.phase, |lib| lib.meta.motion.walk_anim(slot.style, p.phase.into()));
        }
    }
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
            traffic_obstacle: None,
            on_foot_player: None,
            lib: None,
            traffic_models: None,
            lib_rx: None,
            sim_rx: None,
            corridor_hour: None,
            last_hour: f64::NAN,
            gg_ground: None,
            sim: None,
            feed: None,
            feed_failed: false,
            ground,
            structures: world.structures.clone(),
            physical_traffic: Default::default(),
            remote_traffic: Default::default(),
            walker_slots: Vec::new(),
            physical_walkers: Default::default(),
            posed_agents: Vec::new(),
            reset_walkers: false,
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
            // `--crowd`
            city: opts.crowd.then(|| world.city.clone()),
            source: None,
            start_hour: args.time_of_day,
            traffic_hour: args.time_of_day,
            last_focus: None,
            cut_rx: None,
            cut_post: None,
            cuts: 0,
            plazas: Default::default(),
            decks: if opts.on { walker_decks(world, opts.crowd) } else { None },
            bridge_walk: world.bridge_walk.clone(),
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
        // (`--car-materials`) each vehicle material's role and the plates' own coordinates;
        // (`--car-contact`) the bodies' occluder boxes, cooked with them (nothing of the
        // roles draws without `--car-materials`'s shader code); (`--people-motion`) the
        // people's heads held steady in the bake; (`--people-idles`) the wider standing repertoire.
        // (`--car-panels`) the panels' filter and the badges, cooked with the cars' materials.
        let car_materials = args.look.car_materials > 0 || args.look.car_contact > 0;
        let cook = eo_agents::library::CookOptions { far_body: opts.car_detail > 0, car_materials, car_panels: car_materials && eo_render::agents::car_panels(), steady_heads: eo_agents::motion::steady_heads(), more_idles: eo_agents::motion::more_idles() };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-load".into())
            .spawn(move || {
                // (`--people-contact`) the people's contact anchors, made at load (never cached);
                // (`--people-capsules`) their body capsules in their place.
                let loaded = Library::load_opts(&repo, cache.as_deref(), &people, cook).and_then(|mut lib| {
                    let traffic_models = TrafficModels::load(&lib)?;
                    if eo_render::agents::people_capsules() {
                        eo_agents::people::add_capsule_anchors(&mut lib);
                    } else if eo_render::agents::people_contact() {
                        eo_agents::people::add_contact_anchors(&mut lib);
                    }
                    let issue = test_character.as_ref().and_then(|selection| {
                        lib.install_test_player(selection).err().map(|e| format!("test character {}: {e:#}", selection.id))
                    });
                    Ok((lib, traffic_models, issue))
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
        self.feed_failed = false;
        self.last = None;
        self.walker_slots.clear();
        self.physical_walkers.clear();
        self.reset_walkers = true;
        self.last_steps = 0;
        self.last_corridor_steps = 0;
        self.sim_ready_ms = None;
        self.opts.density = density;
        self.corridor_hour = None;
        // `--crowd`: the start hour, and the frontage survey (none without the switch).
        self.cut_rx = None;
        self.cut_post = None;
        self.last_focus = None;
        let crowd = self.opts.crowd.then_some(self.start_hour);
        let traffic_density = car_density_on().then_some(self.traffic_hour); // (`--car-density`; None without it)
        let city = self.city.clone();
        let paths = world.paths.clone();
        let street = world.street.as_ref().map(|s| s.world.clone());
        let plazas_out = self.plazas.clone();
        // The structure level of each way (`--structures`; empty otherwise, which leaves the sim as it was).
        let structures = self.structures.clone();
        let levels = structures.as_ref().map(|s| s.way_levels().clone()).unwrap_or_default();
        let (corridor_traffic, walkers) = corridor::switches();
        if corridor_traffic && self.gg_ground.is_none() {
            let js = eo_landmarks::LandmarksJs::get();
            self.gg_ground = Some(Arc::new(eo_landmarks::set::ground_of(js, "golden-gate-bridge", &|x: f64, z: f64| world.elevation.sample(x, z))));
        }
        let gg_ground = self.gg_ground.clone().filter(|_| corridor_traffic);
        let ground = self.ground.clone();
        // the deck the traffic rides is the drawn bridge's (none with `--no-landmarks`).
        let deck_drawn = world.landmarks.is_some();
        let hour0 = if self.last_hour.is_finite() { self.last_hour } else { 12.0 };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-agents-sim-build".into())
            .spawn(move || {
                let r = (|| -> Result<(SimWorld, Arc<eo_sim::native::NativeSource>)> {
                    let mut source = eo_server::configure_traffic_previews(eo_sim::native::load(&paths)?.with_levels(levels))?;
                    if let Some(sw) = street.as_ref().filter(|_| traffic_street_plan_on()) {
                        source = source.with_traffic_plan(sw.traffic_plan()).with_passing(traffic_passing_on());
                        if let Some(decks) = &structures {
                            source = source.with_deck_surfaces(decks.traffic_surfaces());
                        }
                    }
                    // the corridor layer's ways, read before the local network leaves the roads it drives alone.
                    let connected_decks = !source.levels().is_empty();
                    let corridors = match &gg_ground {
                        Some(gg) => Some(build_corridors(&paths, &source, gg, &ground, walkers && deck_drawn, deck_drawn, hour0, connected_decks)?),
                        None => None,
                    };
                    if corridors.is_some() && !connected_decks {
                        source.without_roads(|r| corridor::EXCLUSIVE.contains(&r.name.as_str()));
                    }
                    if let Some(c) = &city {
                        // `--crowd`: the fronts, the walks of their own and the plazas', the
                        // drawn walks and kerbs, the mapped crosswalks.
                        let mut walks = walk_paths(&paths, c);
                        let mut sections = vec![];
                        let mut zebras = vec![];
                        if let Some(sw) = &street {
                            let rings = plaza_rings(&paths.repo_root, sw);
                            walks.extend(plaza_paths(&rings));
                            *plazas_out.lock().unwrap() = Some(Arc::new(eo_agents::feed::Plazas::new(rings.into_iter().map(|(_, r)| r).collect())));
                            sections = drawn_sections(sw);
                            if people_crosswalks() {
                                zebras = drawn_zebras(sw); // (`--people-crosswalks`) crossed on the paint
                            }
                        }
                        source = source.with_frontage(frontage_edges(c)).with_paths(walks).with_sections(sections).with_crossings(crossing_points(&paths)).with_zebras(zebras);
                    }
                    let source = Arc::new(source);
                    let w = build_sim(&source, focus, density, crowd, traffic_density, corridors, WARMUP_STEPS);
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

    /// Whether startup failed terminally (`lib`, `sim`, or `feed`) so `ready()`
    /// cannot become true without a restart.
    pub fn startup_failed(&self) -> bool {
        if !self.opts.on {
            return false;
        }
        let lib_dead = self.lib.is_none() && self.lib_rx.is_none();
        let sim_dead = !self.remote_mode && self.sim.is_none() && self.sim_rx.is_none();
        lib_dead || sim_dead || self.feed_failed
    }

    /// Wait (pumping) until ready, startup fails terminally, or `timeout_s`.
    pub fn wait_ready(&mut self, ctx: &GpuContext, renderer: &Renderer, focus: [f64; 3], timeout_s: f64) -> bool {
        let t = Instant::now();
        while !self.ready() && !self.startup_failed() && t.elapsed().as_secs_f64() < timeout_s {
            self.pump(ctx, renderer, focus, 1.0 / 60.0);
            if self.startup_failed() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.ready()
    }

    /// Drive the local physics bodies from the same fleet snapshots sent to the feed.
    fn sync_traffic_bodies(&mut self, bodies: &mut TrafficBodies, cw: &mut eo_player::CollisionWorld, near: glam::DVec3, speed: f64, weather: eo_player::WeatherPhysics, physical: bool, props: Option<&crate::props::PropsRuntime>) {
        use eo_player::traffic_contact::{ContactTraffic, TrafficTarget};
        let (Some(sim), Some(lib), Some(models)) = (self.sim.as_mut().filter(|_| self.opts.on && self.last.is_some()), self.lib.as_deref(), self.traffic_models.as_ref()) else {
            ContactTraffic::sync(cw, bodies, near, speed, weather, []);
            return;
        };
        let frame = sim.latest();
        let physical_poses = &self.physical_traffic;
        let nearby = |id: u64, x: f64, z: f64| {
            let in_range = |x: f64, z: f64| (x - near.x).hypot(z - near.z) < eo_player::world::STEP_RADIUS_M;
            in_range(x, z) || physical_poses.get(&id).is_some_and(|(p, _, _)| in_range(p[0].into(), p[2].into()))
        };
        let ground = &self.ground;
        let structures = self.structures.as_deref();
        let regional = frame.regions.iter().enumerate().flat_map(|(ri, r)| {
            r.vehicles.iter().filter_map(move |a| {
                let (x, z) = (r.origin[0] + a.pos[0] as f64, r.origin[1] + a.pos[2] as f64);
                let id = ((ri as u64) << 32) | u64::from(a.id);
                if !nearby(id, x, z) {
                    return None;
                }
                let (s, c) = (a.heading as f64).sin_cos();
                let (chassis, half_base, model) = models.select(lib, a.id, a.kind)?;
                let level = eo_sim::output::level_of_flags(a.flags);
                let (y, pitch) = eo_agents::feed::car_ground_on_route(
                    ground, structures, level, x, z, [s, c], half_base, a.vehicle_surface_y(),
                );
                Some(TrafficTarget { speed: a.speed, model, parked: false, pose: TrafficPose {
                    id,
                    datum: [x as f32, y as f32, z as f32],
                    yaw: a.heading,
                    pitch,
                    chassis,
                }})
            })
        });
        let corridors = frame.corridor.iter().filter_map(|a| {
            let kind = a.kind as usize;
            let id = (1 << 63) | u64::from(a.id);
            if kind >= PRESENTED_LENGTH.len() || !nearby(id, a.pos[0] as f64, a.pos[2] as f64) {
                return None;
            }
            let (chassis, _, model) = models.select(lib, a.id, a.kind)?;
            Some(TrafficTarget { speed: a.speed, model, parked: false, pose: TrafficPose {
                id,
                datum: a.pos,
                yaw: a.heading,
                pitch: a.pitch,
                chassis,
            }})
        });
        if physical {
            let parked = self.parked.iter().filter_map(|a| {
                let id = parked_id(a);
                if !nearby(id, a.pos[0].into(), a.pos[2].into()) { return None; }
                models.parked_target(lib, a, props)
            });
            ContactTraffic::sync(cw, bodies, near, speed, weather, regional.chain(corridors).chain(parked));
        } else {
            bodies.sync(cw, regional.chain(corridors).map(|t| t.pose));
        }
    }

    fn make_feeder(&self, lib: Arc<Library>) -> anyhow::Result<Feeder> {
        let mut feeder = Feeder::new(lib, self.ground.clone(), self.structures.clone(), SEED_INT)?;
        feeder.separate = self.opts.crowd;
        feeder.plazas = self.plazas.lock().unwrap().clone();
        feeder.decks = self.decks.clone();
        feeder.bridge_walk = self.bridge_walk.clone();
        Ok(feeder)
    }

    /// Hold one authored simulation frame for a repeatable contact bench.
    /// Use the normal feed, colliders and upload for every actor in the frame.
    pub fn hold_bench_frame(&mut self, frame: &eo_sim::world::Frame) -> anyhow::Result<()> {
        let lib = self.lib.clone().ok_or_else(|| anyhow::anyhow!("contact bench needs the agent library"))?;
        self.sim = None;
        self.feed = None;
        let mut feeder = self.make_feeder(lib)?;
        let mut instances = vec![];
        feeder.build(frame, &mut instances);
        self.walker_slots = feeder.walker_slots;
        self.last = Some(Arc::new(instances));
        Ok(())
    }

    fn sync_people_bodies(&mut self, people: &mut eo_player::people_contact::PeopleBodies,
        world: &mut eo_player::CollisionWorld, near: glam::Vec3, contacts: bool, impacts: bool) {
        use eo_player::people_contact::{WalkerTarget, WalkingBodies};
        if self.reset_walkers {
            WalkingBodies::sync(world, []);
            self.reset_walkers = false;
        }
        let base = self.last.as_deref().map(Vec::as_slice).unwrap_or(&[]);
        let visible = contacts && self.opts.on && !self.hide_people;
        let targets = self.walker_slots.iter().filter_map(|slot| {
            let a = base.get(slot.index)?;
            let nearby = glam::Vec3::from_array(a.pos).distance_squared(near) <= 24.0 * 24.0;
            let retained = self.physical_walkers.get(&slot.id).is_some_and(|p|
                glam::Vec3::from_array(p.feet).distance_squared(near) <= 36.0 * 36.0);
            if !visible || !impacts || !(nearby || retained) { return None; }
            let forward = glam::Quat::from_array(a.rot) * glam::Vec3::Z;
            Some(WalkerTarget { id: slot.id, feet: slot.feet, drawn: a.pos, model: a.model,
                scale: a.scale, heading: forward.x.atan2(forward.z), speed: a.speed, phase: slot.phase, stride_m: slot.stride_m })
        });
        WalkingBodies::sync(world, targets);
        let proxies = self.walker_slots.iter().filter_map(|slot| {
            let a = base.get(slot.index)?;
            (visible && !world.walking_bodies.contains(slot.id)
                && glam::Vec3::from_array(a.pos).distance_squared(near) <= 24.0 * 24.0)
                .then_some((slot.index, a.pos, a.scale))
        }).collect::<Vec<_>>();
        people.sync(world, proxies);
    }

    pub fn sync_local_bodies(&mut self, vehicles: &mut TrafficBodies, people: &mut eo_player::people_contact::PeopleBodies,
        player: &eo_player::Player, cw: &mut eo_player::CollisionWorld, physical: bool, impacts: bool, props: Option<&crate::props::PropsRuntime>) {
        let feet = player.position(cw);
        let speed = match player.mode {
            eo_player::Mode::Driving(i) => player.vehicles[i].forward_speed(cw),
            eo_player::Mode::OnFoot => player.ground_speed,
        };
        self.sync_traffic_bodies(vehicles,cw,feet,speed,player.weather,physical,props);
        self.sync_people_bodies(people, cw, feet.as_vec3(), physical, impacts);
    }

    pub fn publish_physical_poses(&mut self, cw: &eo_player::CollisionWorld) {
        self.physical_traffic.clear();
        self.physical_traffic.extend(cw.contact_traffic.poses(cw).map(|(id,p,r,v)| (id,(p,r,v))));
        self.physical_walkers.clear();
        self.physical_walkers.extend(cw.walking_bodies.poses(&cw.physics));
    }

    pub fn set_traffic_obstacle(&mut self, player: &eo_player::Player, cw: &eo_player::CollisionWorld) {
        let i = match player.mode {
            eo_player::Mode::Driving(i) => i,
            eo_player::Mode::OnFoot => 0,
        };
        self.on_foot_player = None;
        if player.mode == eo_player::Mode::OnFoot && player.character.grounded {
            let p = player.character.feet();
            if (p.y - self.ground.y(p.x,p.z)).abs() < 0.85 {
                let diameter = 2.0 * eo_player::tuning::capsule::RADIUS_M;
                self.on_foot_player = Some(TrafficObstacle {pos:[p.x,p.z],yaw:player.yaw,level:0,length:diameter,width:diameter});
            }
        }
        self.traffic_obstacle = player.vehicles.get(i).map(|car| {
            let p = car.datum(cw);
            let mut level = 0;
            let mut distance = (p.y - self.ground.y(p.x, p.z)).abs();
            if let Some(structures) = &self.structures {
                for (l, deck) in structures.decks_at(p.x, p.z) {
                    let d = (p.y - deck.y).abs();
                    if d < distance {
                        level = l;
                        distance = d;
                    }
                }
            }
            TrafficObstacle { pos: [p.x, p.z], yaw: car.yaw(cw), length: car.spec.length, width: car.spec.width, level }
        });
    }

    /// Once a frame on the render thread, before `Renderer::update`.
    pub fn pump(&mut self, ctx: &GpuContext, renderer: &Renderer, focus: [f64; 3], dt: f64) -> AgentFrameStats {
        let t0 = Instant::now();
        let mut st = AgentFrameStats::default();
        if let Some(rx) = &self.lib_rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.lib_rx = None;
                    match r {
                        Ok((lib, traffic_models, issue)) => {
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
                            self.traffic_models = Some(traffic_models);
                            self.lib = Some(Arc::new(lib));
                        }
                        Err(e) => {
                            eprintln!("agents: library failed: {e}");
                            self.errors.push(e);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.lib_rx = None;
                    let msg = "agent library loader exited unexpectedly".to_string();
                    eprintln!("agents: {msg}");
                    self.errors.push(msg);
                }
            }
        }
        if let Some(rx) = &self.sim_rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.sim_rx = None;
                    match r {
                        Ok((w, src)) => {
                            self.corridor_hour = w.corridors.as_ref().map(|c| c.hour_handle());
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
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.sim_rx = None;
                    let msg = "agent simulation builder exited unexpectedly".to_string();
                    eprintln!("agents: {msg}");
                    self.errors.push(msg);
                }
            }
        }
        if self.feed.is_none() && !self.feed_failed && (self.sim.is_some() || self.remote_mode) {
            // a remote runtime starts its feed without a local simulation
            if let Some(lib) = &self.lib {
                match self.make_feeder(lib.clone()) {
                    Ok(f) => self.feed = Some(FeedThread::start(f)),
                    Err(e) => {
                        let msg = format!("agent feed failed: {e:#}");
                        eprintln!("agents: {msg}");
                        self.errors.push(msg);
                        self.feed_failed = true;
                    }
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
        self.last_hour = renderer.look.time_of_day();
        if let Some(h) = &self.corridor_hour {
            h.store(self.last_hour.to_bits(), Ordering::Relaxed);
        }
        if let Some(sim) = &mut self.sim {
            sim.post_local(dt, &[(0, Focus { x: focus[0], z: focus[1], yaw: focus[2] })], self.traffic_obstacle, self.on_foot_player);
            let fr = sim.latest();
            self.sim_time = fr.time;
            clock_read = true;
            // a new frame: the regions or the corridor layer stepped (it keeps its own count).
            if fr.steps != self.last_steps || fr.corridor_steps != self.last_corridor_steps {
                self.last_steps = fr.steps;
                self.last_corridor_steps = fr.corridor_steps;
                if let Some(feed) = &self.feed {
                    let seq = feed.post_with_traffic(fr.clone(), self.physical_traffic.clone());
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
                feed.post_with_traffic(fr, std::mem::take(&mut self.remote_traffic));
            }
        }
        if clock_read {
            self.follow_clock(renderer);
        }
        if let Some(feed) = &self.feed {
            let (out, ms) = feed.take_with_walkers();
            if let Some((o, seq)) = out {
                self.walker_slots = o.walkers;
                self.last = Some(o.agents);
                if self.cut_post.is_some_and(|p| p > 0 && seq >= p) {
                    self.physical_walkers.clear();
                    self.reset_walkers = true;
                    self.cut_post = None; // `--crowd`: the cut's street is what is drawn
                }
            }
            st.feed_ms = ms;
        }
        if let Some(g) = gpu {
            if self.opts.on {
                let base: &[GpuAgent] = self.last.as_deref().map(|v| v.as_slice()).unwrap_or(&[]);
                let base = if self.physical_walkers.is_empty() {
                    base
                } else {
                    self.posed_agents.clear();
                    self.posed_agents.extend_from_slice(base);
                    apply_walker_poses(&mut self.posed_agents, &self.walker_slots, &self.physical_walkers, self.lib.as_deref());
                    self.posed_agents.as_slice()
                };
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
                        // (`--people-near`) the people cleared only from round the lens.
                        let near = PEOPLE_NEAR.load(std::sync::atomic::Ordering::Relaxed);
                        let person = |a: &GpuAgent| lib.and_then(|l| l.models.get(a.model as usize)).is_some_and(|m| m.class == eo_agents::CLASS_PERSON);
                        let clear = |a: &GpuAgent, r: f32| if near && person(a) { r.min(PEOPLE_NEAR_CLEAR_M) } else { r };
                        self.scratch.extend(base.iter().filter(|a| !hide.iter().any(|(p, r)| (a.pos[0] - p[0]).hypot(a.pos[2] - p[2]) < clear(a, *r) + body(a))));
                    }
                    if !self.remap_people.is_empty() {
                        for a in self.scratch.iter_mut() {
                            if let Some(&(_, to)) = self.remap_people.iter().find(|(from, _)| *from == a.model) {
                                a.model = to;
                            }
                        }
                    }
                    self.scratch.extend_from_slice(&self.extras);
                    if self.physical_traffic.is_empty() {
                        self.scratch.extend_from_slice(&self.parked);
                    } else {
                        self.scratch.extend(self.parked.iter().map(|original| {
                            let mut car = *original;
                            if let Some(&(pos, rot, speed)) = self.physical_traffic.get(&parked_id(original)) {
                                car.pos = pos;
                                car.rot = rot;
                                car.speed = speed;
                            }
                            car
                        }));
                    }
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
                let _ = tx.send(Ok(build_sim(&src, focus, density, Some(hour), car_density_on().then_some(hour), None, CUT_WARMUP_STEPS)));
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
        let mut agents = self.last.as_deref().cloned().unwrap_or_default();
        apply_walker_poses(&mut agents, &self.walker_slots, &self.physical_walkers, self.lib.as_deref());
        agents
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

    /// (`--car-traffic`, captures) The simulation built again round `eye`
    /// looking along `forward` (`restart_sim`: deterministic, the same
    /// population every run) and waited on until the feed has drawn it; the
    /// simulated agents within `WARM_CLEAR_M` of the eye hidden until
    /// `hide_near` is cleared. Returns the vehicles and people it drew, or
    /// None (nothing warmed: no simulation here, or not in time).
    pub fn warm_traffic(&mut self, ctx: &GpuContext, renderer: &Renderer, world: &World, eye: Vec3, forward: Vec3, hour: f64) -> Option<(usize, usize)> {
        if self.remote_mode || !self.opts.on {
            return None;
        }
        self.traffic_hour = hour; // (`--car-density`) the card's own hour
        let focus = Self::focus_of(eye, forward);
        if self.opts.crowd {
            // `--crowd`: the card's street at the card's hour (the restart starts the
            // clock at `start_hour`; it was the run's start hour for every card).
            self.start_hour = renderer.look.time_of_day();
        }
        self.restart_sim(world, focus, self.opts.density);
        self.hide_near = vec![(eye.to_array(), WARM_CLEAR_M)];
        let t = Instant::now();
        while !self.startup_failed() && t.elapsed().as_secs_f64() < WARM_WAIT_S {
            self.pump(ctx, renderer, focus, 0.0);
            if let (Some(_), Some(last), Some(lib)) = (&self.sim, &self.last, &self.lib) {
                let people = last.iter().filter(|a| lib.models.get(a.model as usize).is_some_and(|m| m.class == eo_agents::CLASS_PERSON)).count();
                return Some((last.len() - people, people));
            }
            if self.startup_failed() {
                eprintln!("agents: warmed traffic aborted after startup error ({:?})", self.errors);
                return None;
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

/// The corridor layer on the drawn deck and streets (`corridor`), starting at `hour`.
fn build_corridors(paths: &eo_data::DataPaths, source: &eo_sim::native::NativeSource, gg: &eo_landmarks::GroundGrid, ground: &Ground, walkers: bool, deck_drawn: bool, hour: f64, connected_decks: bool) -> Result<eo_sim::corridor::Corridors> {
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
    let ways = if connected_decks {
        if walkers { corridor::walk_ways(&deck) } else { Vec::new() }
    } else {
        corridor::ways(source, &surfaces, walkers)
    };
    let c = eo_sim::corridor::Corridors::new(ways, eo_sim::corridor::Params::with_time_gap(time_gap), SEED_INT);
    c.hour_handle().store(hour.to_bits(), Ordering::Relaxed);
    Ok(c)
}

/// A simulation round `focus`: the network within `SIM_RADIUS_M` from the
/// source, one region (`region_config`), `steps` fixed steps run.
fn build_sim(source: &eo_sim::native::NativeSource, focus: [f64; 3], density: u32, crowd: Option<f64>, traffic: Option<f64>, corridors: Option<eo_sim::corridor::Corridors>, steps: usize) -> SimWorld {
    let tiles = source.resident_tiles(focus[0], focus[1], SIM_RADIUS_M);
    let mut net = source.new_network();
    net.add(source.build(&tiles));
    let mut w = SimWorld::new(net);
    w.corridors = corridors;
    let f = Focus { x: focus[0], z: focus[1], yaw: focus[2] };
    let r = Region::new(&w.net, 0, region_config(density, crowd, traffic), f);
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
            if let Some(mut c) = eo_agents::feed::player_car(lib, v.spec.config.model.body(), [datum.x, datum.y, datum.z], [rot.x, rot.y, rot.z, rot.w], self.spins[k], steer, speed, braking, PLAYER_CAR_PAINT) {
                if let Some((i, fraction)) = door && i == k {
                    c.clip_phase = fraction;
                }
                out.push(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hatchback_contact_uses_the_body_drawn_by_the_fleet() {
        use eo_player::traffic_contact::{ContactTraffic, TrafficTarget};
        use eo_player::{BuildingSet, CollisionWorld, GroundProvider, Player, StepInput};
        use eo_sim::output::{AgentBuffer, AgentInstance, KIND_SEDAN};
        use glam::DVec3;

        struct Flat;
        impl GroundProvider for Flat {
            fn ground_y(&self, _: f64, _: f64) -> f64 { 0.0 }
        }

        let paths = eo_data::DataPaths::discover().unwrap();
        let lib = Arc::new(Library::load_with(&paths.repo_root, None, &eo_agents::library::PeopleSource::Off).unwrap());
        let models = TrafficModels::load(&lib).unwrap();
        let hatch = lib.model("compactHatch").expect("shipped hatchback");
        let id = (1..256).find(|&id| {
            lib.traffic_model(&models.catalog, SEED_INT, id, KIND_SEDAN).is_some_and(|(_, model)| model == hatch)
        }).expect("an ambient hatchback");
        let route = AgentInstance { id, pos: [0.0, 0.0, 8.0], ..Default::default() };
        let frame = eo_sim::world::Frame {
            regions: vec![AgentBuffer { vehicles: vec![route], ..Default::default() }],
            ..Default::default()
        };
        let mut feeder = Feeder::new(lib.clone(), Ground::Flat, None, SEED_INT).unwrap();
        let mut drawn = vec![];
        feeder.build(&frame, &mut drawn);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].model, hatch);

        let (chassis, _, model) = models.select(&lib, route.id, route.kind).unwrap();
        let (min, max) = lib.vehicle_chassis_bounds(drawn[0].model).unwrap();
        assert_eq!(chassis.length(), max.z * drawn[0].scale - min.z * drawn[0].scale);
        let expected = f64::from(drawn[0].pos[2] + min.z * drawn[0].scale)
            - eo_player::tuning::capsule::RADIUS_M - eo_player::tuning::capsule::CONTROLLER_OFFSET_M;

        for dynamic in [false, true] {
            let mut world = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
            world.load_blocking(DVec3::ZERO, eo_player::world::STEP_RADIUS_M);
            let mut player = Player::new(DVec3::ZERO, 0.0);
            let mut traffic = TrafficBodies::default();
            let walk = StepInput { move_y: 1.0, ..Default::default() };
            for _ in 0..480 {
                let pose = TrafficPose { id: u64::from(id), datum: drawn[0].pos, yaw: route.heading, pitch: 0.0, chassis };
                if dynamic {
                    ContactTraffic::sync(&mut world, &mut traffic, player.character.feet(), player.ground_speed,
                        Default::default(), [TrafficTarget { pose, speed: 0.0, model, parked: false }]);
                } else {
                    traffic.sync(&mut world, [pose]);
                }
                player.fixed_step(&mut world, &walk);
            }
            let stop = player.character.feet().z;
            if dynamic {
                let mass = world.physics.bodies.iter().find(|(_, body)| body.is_dynamic()).unwrap().1.mass();
                let hatchback = eo_player::vehicle::VehicleConfig { model: eo_player::vehicle::VehicleModel::Hatchback, ..Default::default() }.spec();
                assert!((f64::from(mass) - hatchback.curb_mass_kg).abs() < 0.1,
                    "drawn hatchback has a {mass:.1} kg rigid body; expected {:.1} kg", hatchback.curb_mass_kg);
            }
            assert!((stop - expected).abs() < 0.04, "dynamic={dynamic}: stops at {stop:.3} m, expected {expected:.3} m");
            eprintln!("hatchback dynamic={dynamic}: stops at {stop:.3} m, expected {expected:.3} m");
        }
    }

    #[test]
    fn walker_render_pose_follows_identity_across_slot_reuse() {
        use eo_agents::feed::WalkerSlot;
        use eo_player::people_contact::WalkerPose;
        let slot = |id, index| WalkerSlot { id, index, feet: [7., 0., 8.], style: 0, phase: 0., stride_m: 1.4 };
        let pose = WalkerPose { model: 5, route_feet: [7., 0., 8.], feet: [8., 0.1, 9.],
            rotation: glam::Quat::from_rotation_z(0.7).to_array(), walking_speed: 0.2, phase: 0.24 };
        let poses = [(41, pose)].into_iter().collect();
        let original = GpuAgent { model: 5, ..Default::default() };
        let mut bodies = [original; 2];
        apply_walker_poses(&mut bodies, &[slot(41, 0), slot(42, 1)], &poses, None);
        let struck = bodies[0];
        assert_eq!((struck.pos, struck.rot, struck.speed, struck.anim), (pose.feet, pose.rotation, pose.walking_speed, pose.phase));
        bodies = [original; 2];
        apply_walker_poses(&mut bodies, &[slot(42, 0), slot(41, 1)], &poses, None);
        assert_eq!(bodies, [original, struck]);
        bodies = [original; 2];
        let distant = WalkerSlot { feet: [80., 0., 8.], ..slot(41, 0) };
        apply_walker_poses(&mut bodies, &[distant], &poses, None);
        assert_eq!(bodies, [original; 2]);
    }

    #[test]
    fn the_drawn_vehicle_selects_its_mass_within_a_mixed_traffic_class() {
        use eo_player::traffic_contact::{ContactTraffic, TrafficTarget};
        use eo_player::vehicle::{VehicleConfig, VehicleModel};
        use eo_player::{BuildingSet, CollisionWorld, GroundProvider};
        use eo_sim::output::{KIND_BUS, KIND_SEDAN, KIND_TAXI, KIND_TRUCK};
        use glam::DVec3;

        struct Flat;
        impl GroundProvider for Flat {
            fn ground_y(&self, _: f64, _: f64) -> f64 { 0. }
        }
        let paths = eo_data::DataPaths::discover().unwrap();
        let mut lib = Library::load_with(&paths.repo_root, None, &eo_agents::library::PeopleSource::Off).unwrap();
        let models = TrafficModels::load(&lib).unwrap();
        let mut world = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
        world.load_blocking(DVec3::ZERO, eo_player::world::STEP_RADIUS_M);
        let mut scripted = TrafficBodies::default();
        let curb = |model| VehicleConfig { model, ..Default::default() }.spec().curb_mass_kg;
        for (kind, body, expected) in [
            (KIND_SEDAN, "compactHatch", curb(VehicleModel::Hatchback)),
            (KIND_SEDAN, "sedan", curb(VehicleModel::Sedan)),
            (KIND_TRUCK, "deliveryVan", curb(VehicleModel::DeliveryVan)),
            (KIND_SEDAN, "compactHatchSport", curb(VehicleModel::Hatchback)),
            (KIND_SEDAN, "compactHatchCross", curb(VehicleModel::Hatchback)),
            (KIND_SEDAN, "sedanSport", curb(VehicleModel::Sedan)),
            (KIND_SEDAN, "sedanLuxe", curb(VehicleModel::Sedan)),
            (KIND_SEDAN, "coupe", 1276.86),
            (KIND_SEDAN, "wagon", 1710.04),
            (KIND_SEDAN, "compactSuv", 1606.62),
            (KIND_SEDAN, "minivan", 1568.07),
            (KIND_TAXI, "taxi", 1663.78),
            (KIND_TAXI, "taxiCheckered", 1663.78),
            (KIND_TAXI, "taxiAdBoard", 1663.78),
            (KIND_TRUCK, "pickup", 2025.29),
            (KIND_TRUCK, "fullSizeSuv", 2510.18),
            (KIND_TRUCK, "boxTruck", 3991.61),
            (KIND_BUS, "cityBus", 12578.12),
        ] {
            let id = (0..512).find(|&id| lib.traffic_model(&models.catalog, SEED_INT, id, kind)
                .is_some_and(|(_, m)| lib.meta.model_names[m as usize] == body)).expect("body in the shipped fleet");
            let (chassis, _, model) = models.select(&lib, id, kind).unwrap();
            for parked in [false, true] {
                let pose = TrafficPose { id: 1, datum: [0.; 3], yaw: 0., pitch: 0., chassis };
                let target = TrafficTarget { pose, model, speed: 0., parked };
                ContactTraffic::sync(&mut world, &mut scripted, DVec3::ZERO, 0., Default::default(), [target]);
                let mut dynamic = world.physics.bodies.iter().filter(|(_, b)| b.is_dynamic());
                let mass = dynamic.next().expect("active traffic body").1.mass();
                assert!(dynamic.next().is_none(), "the reused slot retained the previous body");
                assert!((f64::from(mass) - expected).abs() < 0.1, "{body}, parked={parked}, rigid-body mass {mass} kg, reference {expected} kg");
                eprintln!("traffic {body}, parked={parked}: {mass:.1} kg");
            }
        }
        // Include the special bodies outside the four ordinary moving traffic classes.
        for (body, expected_kg) in [("patrolSedan", 1982.65), ("patrolSuv", 2119.18), ("exoticCoupe", 1451.04)] {
            let index = lib.model(body).expect("shipped special body");
            let instance = GpuAgent { model: index, scale: eo_data::groundscale::vehicles() as f32, rot: glam::Quat::IDENTITY.to_array(), ..Default::default() };
            let target = models.parked_target(&lib, &instance, None).expect("parked mesh contact");
            assert!(matches!(target.pose.chassis, TrafficChassis::Mesh { .. }));
            ContactTraffic::sync(&mut world, &mut scripted, DVec3::ZERO, 0., Default::default(), [target]);
            let mut bodies = world.physics.bodies.iter().filter(|(_, b)| b.is_dynamic());
            let mass = bodies.next().unwrap().1.mass();
            assert!(bodies.next().is_none());
            assert!((mass - expected_kg).abs() < 0.1, "{body}: {mass} kg vs {expected_kg} kg");
        }
        lib.meta.vehicle_models.retain(|name, _| name == "sedan");
        let (_, _, fallback) = models.select(&lib, 1, KIND_BUS).unwrap();
        assert_eq!(fallback, eo_player::traffic_contact::TrafficModel::from_body("sedan"), "partial library draws its fallback sedan");
    }

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
            let _ = build_sim(&source, [x, z, 0.0], 1, Some(hour), None, None, 60);
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

    #[test]
    fn startup_failed_detects_terminal_loader_and_feed_failures() {
        let make_rt = |on: bool, remote_mode: bool| AgentRuntime {
            opts: AgentOpts { on, ..Default::default() },
            traffic_obstacle: None,
            on_foot_player: None,
            lib: None,
            traffic_models: None,
            lib_rx: None,
            sim_rx: None,
            corridor_hour: None,
            last_hour: f64::NAN,
            gg_ground: None,
            sim: None,
            feed: None,
            feed_failed: false,
            ground: Ground::Flat,
            structures: None,
            physical_traffic: Default::default(),
            remote_traffic: Default::default(),
            walker_slots: Vec::new(),
            physical_walkers: Default::default(),
            posed_agents: Vec::new(),
            reset_walkers: false,
            last: None,
            last_steps: 0,
            last_corridor_steps: 0,
            scratch: vec![],
            extras: vec![],
            test_rows: vec![],
            parked: vec![],
            hide_near: vec![],
            remap_people: vec![],
            started: Instant::now(),
            sim_ready_ms: None,
            lib_ready_ms: None,
            errors: vec![],
            sim_time: 0.0,
            remote: None,
            remote_mode,
            hide_people: false,
            clock: None,
            city: None,
            source: None,
            start_hour: 12.0,
            traffic_hour: 12.0,
            last_focus: None,
            cut_rx: None,
            cut_post: None,
            cuts: 0,
            plazas: Default::default(),
            decks: None,
            bridge_walk: None,
        };

        // Disabled runtime is immediately ready and never failed.
        let off = make_rt(false, false);
        assert!(off.ready() && !off.startup_failed());

        // Active loaders in flight: not failed.
        let mut loading = make_rt(true, false);
        let (_lib_tx, lib_rx) = std::sync::mpsc::channel();
        let (_sim_tx, sim_rx) = std::sync::mpsc::channel();
        loading.lib_rx = Some(lib_rx);
        loading.sim_rx = Some(sim_rx);
        assert!(!loading.ready() && !loading.startup_failed());

        // Sim loader failed (channel cleared with sim=None): startup_failed immediately aborts wait_ready!
        loading.sim_rx = None;
        assert!(loading.startup_failed());

        // Remote mode has no local sim; only lib loader or feed failure is terminal.
        let mut remote = make_rt(true, true);
        let (_r_tx, r_rx) = std::sync::mpsc::channel();
        remote.lib_rx = Some(r_rx);
        assert!(!remote.startup_failed());
        remote.feed_failed = true;
        assert!(remote.startup_failed());
    }
}
