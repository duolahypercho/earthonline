//! Earth Online native: what stands on and beside the street, placed by the
//! browser game's own rules (ported; no GPU here).
//!
//! * `signals`  traffic signal poles, mast arms, vehicle heads and pedestrian
//!              heads at signalised junctions (`planTrafficSignalHeads`,
//!              `planPedestrianCrossings`, `planPedestrianHeads`), with the
//!              junction and bearing each head shows, so the app can light it
//!              from eo-sim's live controllers.
//! * `signs`    the SFMTA sign inventory on its posts (`planStreetSigns` with
//!              `createStreetSignKerbSnap`).
//! * `trees`    the DataSF street tree list on the kerb (`placeSurveyedTrees`).
//! * `pads`     detectable-warning pads on the kerb ramps the street cuts.
//! * `parking`  parked cars along the kerb (`planParkedVehicles`).
//! * `roadway`  the shared roadway / ramp index those rules ask.
//!
//! Every California number comes from `eo_data::CaStandards` (`caMetric`);
//! every JS design value from `data/props-js.json.gz`
//! (the earlier browser version's exporter). Placement is per 128 m street tile
//! (`PropsWorld::plan_tile`), pure and deterministic: nothing depends on the
//! camera or on which tiles were asked before.

pub mod data;
pub mod eztree;
pub mod furniture;
pub mod overhead;
pub mod pads;
pub mod parking;
pub mod parking_rules;
pub mod roadway;
pub mod sign_atlas;
pub mod signals;
pub mod signs;
pub mod trees;

use anyhow::Result;
use eo_data::CaStandards;
use eo_street::StreetWorld;
use eo_street::plan::{Entry, Frame, frame_at};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use data::JsProps;
pub use roadway::RoadwayIndex;

pub type P2 = [f64; 2];

/// What a placed prop draws as (the renderer's model table follows this order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Model {
    /// A round signal pole, `scale.y` = height (mast or pedestal or pedestrian post).
    SignalPole = 0,
    /// A mast arm, local +x along the arm, `scale.x` = length.
    MastArm = 1,
    /// A vehicle signal head (the JS `signalHead-v1.glb` body + three lenses).
    SignalHead = 2,
    /// A pedestrian signal head (housing + hand / person faces).
    PedHead = 3,
    /// A sign post pipe, `scale.y` = height.
    SignPost = 4,
    /// A sign plate, `scale` = (width, height, 1); `variant` = the face index.
    SignPlate = 5,
    /// A street tree (the furniture walk's `tree` item); `scale` = (scale, heightScale, scale)
    /// of the JS item, `variant` = its street species index (`STREET_TREE_SPECIES`;
    /// `eztree::street::street_tree_species_id(variant, 0, mode)` names it). // props2
    Tree = 6,
    /// A palm (`palm-v1.glb`); `scale` = (scale, heightScale, scale) of the JS item. // props2
    Palm = 7,
    /// A detectable-warning pad, `scale` = (width along the kerb, 1, depth).
    TactilePad = 8,
    // the street-furniture kinds (furniture.rs), one each, in catalogue
    // order. `scale` = (scale, heightScale, scale) of the JS item, `variant` = its
    // variant (bollard: cast iron / steel; mailbox: collection / relay), yaw its rotation.
    Hydrant = 9,
    ParkingMeter = 10,
    PayStation = 11,
    /// The corner sign pole (the walk's `signPole`, which carries the street-name blades).
    SignPole = 12,
    Bollard = 13,
    WasteBin = 14,
    BikeRack = 15,
    NewsBox = 16,
    Mailbox = 17,
    /// The district street lights (`lampHistoric-v1`, `lampBanner-v1`, `lampCobra-v1` GLBs); see `TilePlacement::lights`.
    LampHistoric = 18,
    LampBanner = 19,
    LampCobra = 20,
    Planter = 21,
    Bench = 22,
    TransitShelter = 23,
    BusStopFlag = 24,
    Standpipe = 25,
    WallMeter = 26,
    CafeSet = 27,
    SandwichBoard = 28,
    UtilityBox = 29,
    Scaffold = 30,
    Barricade = 31,
    PosterHoarding = 32,
    PedSignalPost = 33,
    SignalCabinet = 34,
}

pub const MODEL_COUNT: usize = 35;

impl Model {
    /// The model a street-furniture kind (`STREET_FURNITURE_KINDS` id) draws as; None for the
    /// fixture dressing kinds the walk never places (`lampPost`, `lampGlobe`, `signalHead`). // props2
    pub fn from_furniture_kind(id: &str) -> Option<Model> {
        Some(match id {
            "hydrant" => Model::Hydrant,
            "parkingMeter" => Model::ParkingMeter,
            "payStation" => Model::PayStation,
            "signPole" => Model::SignPole,
            "bollard" => Model::Bollard,
            "wasteBin" => Model::WasteBin,
            "bikeRack" => Model::BikeRack,
            "newsBox" => Model::NewsBox,
            "mailbox" => Model::Mailbox,
            "tree" => Model::Tree,
            "palm" => Model::Palm,
            "lampHistoric" => Model::LampHistoric,
            "lampBanner" => Model::LampBanner,
            "lampCobra" => Model::LampCobra,
            "planter" => Model::Planter,
            "bench" => Model::Bench,
            "transitShelter" => Model::TransitShelter,
            "busStopFlag" => Model::BusStopFlag,
            "standpipe" => Model::Standpipe,
            "wallMeter" => Model::WallMeter,
            "cafeSet" => Model::CafeSet,
            "sandwichBoard" => Model::SandwichBoard,
            "utilityBox" => Model::UtilityBox,
            "scaffold" => Model::Scaffold,
            "barricade" => Model::Barricade,
            "posterHoarding" => Model::PosterHoarding,
            "pedSignalPost" => Model::PedSignalPost,
            "signalCabinet" => Model::SignalCabinet,
            _ => return None,
        })
    }
}

/// Live state an instance shows (resolved by the app to a lamp channel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Live {
    None,
    /// A vehicle head: the junction at `node` (its plan position) and the bearing of the traffic it holds.
    Vehicle { node: P2, bearing: f64 },
    /// A pedestrian head: the junction, the crossing's bearing and its length.
    Pedestrian { node: P2, bearing: f64, length_m: f64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub model: Model,
    /// World position of the model origin (its foot, or a head's centre).
    pub pos: [f64; 3],
    /// three.js `rotation.y`: local +Z faces (sin yaw, cos yaw).
    pub yaw: f64,
    pub scale: [f32; 3],
    pub variant: u16,
    pub live: Live,
}

/// A parked car (static; drawn with eo-agents' vehicle library).
#[derive(Clone, Debug)]
pub struct ParkedCar {
    /// The JS catalogue id (`VEHICLE_SPECS[].id`).
    pub spec: String,
    /// Body centre on the ground: x, ground y, z.
    pub pos: [f64; 3],
    /// three.js yaw of the body (+Z forward).
    pub yaw: f64,
    /// Paint 0xrrggbb.
    pub paint: u32,
    pub entry: u32,
    pub side: i8,
    pub station: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TilePlacement {
    pub props: Vec<Prop>,
    pub parked: Vec<ParkedCar>,
    /// The district street lights' luminaires (one per lamp prop, owned by its foot). // props2
    pub lights: Vec<furniture::LampLight>,
}

/// The California values the rules read (`caMetric`), named.
#[derive(Clone, Debug)]
pub struct Standards {
    pub sign_mount_min: f64,
    pub sign_mount_secondary_min: f64,
    pub sign_lateral_typical: f64,
    pub sign_lateral_limited: f64,
    pub throughway_min: f64,
    pub hydrant_clear: f64,
    pub signal_approach_near_side: f64,
    pub signal_view_min: f64,
    pub detectable_warning_depth: f64,
    pub no_parking_driveway: f64,
    pub no_parking_daylighting: f64,
}

impl Standards {
    pub fn load(ca: &CaStandards) -> Result<Self> {
        Ok(Self {
            sign_mount_min: ca.metric_key("sign-mounting-height-urban", "min")?,
            sign_mount_secondary_min: ca.metric_key("sign-mounting-height-urban", "secondarySignMin")?,
            sign_lateral_typical: ca.metric_key("sign-lateral-offset-urban", "typicalFromCurbFace")?,
            sign_lateral_limited: ca.metric_key("sign-lateral-offset-urban", "minWhereLimited")?,
            throughway_min: ca.metric_key("sidewalk-throughway-zone-sf", "alleyOrAdaMin")?,
            hydrant_clear: ca.metric("no-parking-fire-hydrant")?,
            signal_approach_near_side: ca.metric_key("no-parking-signalized-approach", "nearSide")?,
            signal_view_min: ca.metric_key("signal-face-longitudinal-position", "min")?,
            detectable_warning_depth: ca.metric_key("detectable-warning-size", "depthMin")?,
            no_parking_driveway: ca.metric("no-parking-driveway")?,
            no_parking_daylighting: ca.metric_key("no-parking-crosswalk-daylighting", "approachSide")?,
        })
    }
}

/// `sidewalkBand`: the furnishable footway band of one side of a ribbon.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub side: f64,
    pub walk: f64,
    pub inner: f64,
    pub outer: f64,
    pub usable: f64,
    pub clear: f64,
}

pub struct PropsWorld {
    pub street: Arc<StreetWorld>,
    pub js: Arc<JsProps>,
    pub std: Standards,
    pub roadway: RoadwayIndex,
    pub seed: String,
    tree_cells: HashMap<(i32, i32), Vec<u32>>,
    sign_cells: HashMap<(i32, i32), Vec<u32>>,
    sign_plate_start: Vec<u32>,
    /// Signalised plan nodes by tile.
    signal_nodes: HashMap<(i32, i32), Vec<u32>>,
    entry_cache: Mutex<HashMap<u32, Arc<EntryProps>>>,
    snap_cells: std::sync::OnceLock<HashMap<(i32, i32), Vec<u32>>>,
    mouths: std::sync::OnceLock<HashMap<(u32, i8), Vec<[f64; 2]>>>,
    pub(crate) kerb_rules: std::sync::OnceLock<parking_rules::KerbRules>,
    /// The ribbon each list tree belongs to (nearest centreline), computed once.
    tree_owner: Mutex<HashMap<u32, u32>>,
    /// The street-furniture walk's world (built on first use). // props2
    furniture: std::sync::OnceLock<Arc<furniture::Furniture>>,
    pub build_ms: f64,
}

/// What one ribbon places (cached: a ribbon crosses several tiles).
#[derive(Default)]
pub struct EntryProps {
    pub props: Vec<Prop>,
    pub parked: Vec<ParkedCar>,
    pub lights: Vec<furniture::LampLight>,
}

/// `SURVEYED_CELL` of street-furniture.js (the tree list's index cell).
pub const SURVEYED_CELL_M: f64 = 32.0;

impl PropsWorld {
    pub fn new(street: Arc<StreetWorld>, js: Arc<JsProps>, std: Standards) -> Self {
        let t0 = std::time::Instant::now();
        let roadway = RoadwayIndex::new(&street.plan, &street.o);
        let seed = js.consts["seedTag"].as_str().unwrap_or("city").to_string();
        let mut tree_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let st = js.trees.stride;
        for i in 0..js.trees.data.len() / st {
            let (x, z) = (js.trees.data[i * st] as f64 / 10.0, js.trees.data[i * st + 1] as f64 / 10.0);
            tree_cells.entry(((x / SURVEYED_CELL_M).floor() as i32, (z / SURVEYED_CELL_M).floor() as i32)).or_default().push(i as u32);
        }
        let cell = signs::SIGN_CELL_M(&js);
        let mut sign_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut sign_plate_start = vec![];
        let mut p = 0u32;
        for i in 0..js.signs.posts.len() / 4 {
            sign_plate_start.push(p);
            p += js.signs.posts[i * 4 + 3] as u32;
            let (x, z) = (js.signs.posts[i * 4] as f64 / 10.0, js.signs.posts[i * 4 + 1] as f64 / 10.0);
            sign_cells.entry(((x / cell).floor() as i32, (z / cell).floor() as i32)).or_default().push(i as u32);
        }
        let mut signal_nodes: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (ni, n) in street.plan.nodes.iter().enumerate() {
            if n.signalised() {
                signal_nodes.entry(eo_street::tile_key(n.position[0], n.position[1])).or_default().push(ni as u32);
            }
        }
        let mut w = Self {
            street,
            js,
            std,
            roadway,
            seed,
            tree_cells,
            sign_cells,
            sign_plate_start,
            signal_nodes,
            entry_cache: Mutex::new(HashMap::new()),
            snap_cells: std::sync::OnceLock::new(),
            mouths: std::sync::OnceLock::new(),
            kerb_rules: std::sync::OnceLock::new(),
            tree_owner: Mutex::new(HashMap::new()),
            furniture: std::sync::OnceLock::new(),
            build_ms: 0.0,
        };
        w.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        w
    }

    /// Every street tile key with anything to place.
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        self.street.tile_keys()
    }

    /// Everything placed in one 128 m street tile (owned by position).
    pub fn plan_tile(&self, key: (i32, i32)) -> TilePlacement {
        let mut out = TilePlacement::default();
        let inside = |x: f64, z: f64| eo_street::tile_key(x, z) == key;
        // Ribbon props (trees, pads, parked cars) from every ribbon near the tile.
        let mut entries: Vec<u32> = vec![];
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(t) = self.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                    entries.extend(t.spans.iter().map(|s| s.0));
                }
            }
        }
        entries.sort_unstable();
        entries.dedup();
        for ei in entries {
            let ep = self.entry_props(ei);
            out.props.extend(ep.props.iter().filter(|p| inside(p.pos[0], p.pos[2])).copied());
            out.parked.extend(ep.parked.iter().filter(|c| inside(c.pos[0], c.pos[2])).cloned());
            out.lights.extend(ep.lights.iter().filter(|l| inside(l.base[0], l.base[2])).copied());
        }
        // Junction props: corner pads and signals owned by the node's tile.
        if let Some(t) = self.street.tiles.get(&key) {
            for &ni in &t.nodes {
                pads::node_pads(self, ni as usize, &mut out.props);
            }
        }
        if let Some(nodes) = self.signal_nodes.get(&key) {
            for &ni in nodes {
                signals::plan_node(self, ni as usize, &mut out.props);
            }
        }
        out.props.extend(signs::plan_tile(self, key));
        out
    }

    fn entry_props(&self, ei: u32) -> Arc<EntryProps> {
        if let Some(e) = self.entry_cache.lock().unwrap().get(&ei) {
            return e.clone();
        }
        let mut ep = EntryProps::default();
        let e = &self.street.plan.entries[ei as usize];
        pads::entry_pads(self, e, &mut ep.props);
        // the furniture walk (trees, palms and every other kind, lamps with their lights).
        let fe = self.furniture().entry(self, ei as usize);
        ep.props.extend(fe.props.iter().copied());
        ep.lights.extend(fe.lights.iter().copied());
        ep.parked = parking::plan_entry(self, ei as usize);
        let ep = Arc::new(ep);
        let mut c = self.entry_cache.lock().unwrap();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(ei, ep.clone());
        ep
    }

    /// The street-furniture walk's world (its JS export and the frontage attributes of the
    /// city's buildings, read on first use). // props2
    pub fn furniture(&self) -> &Arc<furniture::Furniture> {
        self.furniture.get_or_init(|| Arc::new(furniture::Furniture::new(self).expect("street furniture world")))
    }

    /// Build the furniture world from buildings already loaded (skips reading the city again). // props2
    pub fn install_furniture(&self, buildings: &[eo_data::Building]) -> Result<()> {
        let f = furniture::Furniture::with_buildings(self, furniture::data::FurnitureJs::load()?, buildings)?;
        let _ = self.furniture.set(Arc::new(f));
        Ok(())
    }

    /// The drawn street surface (paving, else the ground carpet) at (x, z).
    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.street.surface_at(x, z, None)
    }

    /// `sidewalkBand(seg, side, o, clear)`.
    pub fn band(&self, e: &Entry, side: f64, clear_width: f64) -> Option<Band> {
        let o = &self.street.o;
        let walk = if side > 0.0 { e.walks[0] } else { e.walks[1] };
        if !(walk >= o.min_sidewalk_width) {
            return None;
        }
        let inner = e.half + o.curb_top_width;
        let outer = e.half + walk;
        let usable = outer - inner;
        if !(usable > 0.25) {
            return None;
        }
        let clear = usable.min(clear_width.max(self.std.throughway_min));
        Some(Band { side, walk, inner, outer, usable, clear })
    }

    pub fn segment_of(&self, e: &Entry) -> &eo_street::city::Segment {
        &self.street.city.segments[e.road]
    }
}

/// The street world as eo-app builds it (tests, examples, census).
pub fn load_street_world(paths: &eo_data::DataPaths) -> Result<Arc<StreetWorld>> {
    let city = eo_data::City::load(paths)?;
    let grid = Arc::new(eo_data::ElevationGrid::load(paths)?);
    let o = eo_street::StreetOptions::load()?;
    let sc = eo_street::StreetCity::load()?;
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    Ok(Arc::new(StreetWorld::from_plan(o, sc, plan, eo_street::Terrain { grid, carpet_lift: lift }, eo_street::BuildingIndex::new(rings))))
}

/// The whole props world from the repo's data.
pub fn load_world(paths: &eo_data::DataPaths) -> Result<PropsWorld> {
    let street = load_street_world(paths)?;
    let std = Standards::load(&CaStandards::load(paths)?)?;
    Ok(PropsWorld::new(street, Arc::new(JsProps::load()?), std))
}

/// `streetStationAt(seg, s, allowMiter)`.
pub fn station_at(e: &Entry, s: f64, miter: bool) -> Frame {
    frame_at(&e.points, &e.cum, s.clamp(0.0, e.length), miter)
}

/// `projectPointOntoSegment` (street-furniture.js): station, distance and lateral (+n positive).
pub fn project(e: &Entry, x: f64, z: f64) -> Option<(f64, f64, f64)> {
    let mut station = 0.0;
    let mut best = f64::INFINITY;
    for i in 0..e.points.len().saturating_sub(1) {
        let (a, b) = (e.points[i], e.points[i + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        if l2 < 1e-9 {
            continue;
        }
        let t = (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0);
        let d = (x - (a[0] + dx * t)).hypot(z - (a[1] + dz * t));
        if d >= best {
            continue;
        }
        best = d;
        station = e.cum[i] + l2.sqrt() * t;
    }
    if !best.is_finite() {
        return None;
    }
    let st = station_at(e, station, false);
    Some((station, best, (x - st.x) * st.nx + (z - st.z) * st.nz))
}

/// `streetRandom(seed)`: the JS street's seeded 0..1 stream (mulberry32 on FNV-1a).
pub struct StreetRandom(u32);

impl StreetRandom {
    pub fn new(seed: &str) -> Self {
        StreetRandom(eo_street::hash32(seed).wrapping_add(0x6d2b79f5))
    }
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let t = self.0;
        let mut r = (t ^ (t >> 15)).wrapping_mul(1 | t);
        r = r.wrapping_add((r ^ (r >> 7)).wrapping_mul(61 | r)) ^ r;
        (r ^ (r >> 14)) as f64 / 4294967296.0
    }
}

/// `pickWeighted(list, draw, weightOf)` (vehicle-catalogue.js).
pub fn pick_weighted<T>(list: &[T], draw: f64, weight: impl Fn(&T) -> f64) -> Option<&T> {
    let total: f64 = list.iter().map(|e| weight(e).max(0.0)).sum();
    if !(total > 0.0) {
        return list.first();
    }
    let mut t = draw.clamp(0.0, 0.999999) * total;
    for e in list {
        t -= weight(e).max(0.0);
        if t <= 0.0 {
            return Some(e);
        }
    }
    list.last()
}

/// Seconds a plan for every tile takes, and counts (census / tests).
pub fn census(w: &PropsWorld, keys: &[(i32, i32)]) -> (HashMap<Model, usize>, usize) {
    let mut by = HashMap::new();
    let mut parked = 0;
    for &k in keys {
        let t = w.plan_tile(k);
        for p in &t.props {
            *by.entry(p.model).or_insert(0) += 1;
        }
        parked += t.parked.len();
    }
    (by, parked)
}
