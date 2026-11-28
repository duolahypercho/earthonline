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
//! * `clearance` with the grade-separated roads (`--structures`): nothing
//!              placed pierces a deck, girder, bent or barrier, the columns
//!              keep cars and furniture off their footprint, and the built
//!              freeway decks get barrier-mounted luminaires.
//!
//! Every California number comes from `eo_data::CaStandards` (`caMetric`);
//! every JS design value from `data/props-js.json.gz`
//! (the earlier browser version's exporter). Placement is per 128 m street tile
//! (`PropsWorld::plan_tile`), pure and deterministic: nothing depends on the
//! camera or on which tiles were asked before.

pub mod basins; // street tree basins (`--tree-basins`)
pub mod boats; // moored boats at the floating docks' berths (eo-app `--no-boats`)
pub mod clearance;
pub mod data;
pub mod eztree;
pub mod furniture;
pub mod overhead;
pub mod pads;
pub mod props_v2; // street props v2 (eo-app `--no-street-props-v2`)
pub mod parking;
pub mod parking_rules;
pub mod roadway;
pub mod sign_atlas;
pub mod shop_lettering; // the shop signs' words from the system's fonts (eo-kit `--facade-clutter`, stage S1)
pub mod signal_models; // the signal heads as photographed models (eo-app `--no-signal-models`)
pub mod signal_poles; // the signal poles, mast arms and head mounts as Blender models (eo-app `--no-signal-poles`)
pub mod signal_lamps; // street lights on the signal mast poles (eo-app `--no-signal-luminaires`)
pub mod signals;
pub mod sign_models; // the SFMTA sign plates as Blender models (eo-app `--no-sign-models`)
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
    /// A street tree's basin (`basins.rs`, `--tree-basins`): a flat panel, `scale` = (across
    /// the walk, 1, along the kerb), local +Z along the kerb; `variant` = the grade along
    /// the kerb as mm per m + 1000 (props_types.wgsl `PR_TREE_BASIN` tilts it, as the pad).
    TreeBasin = 35,
}

pub const MODEL_COUNT: usize = 36;

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
    /// The signalised junctions' name signs and push buttons (`signals::Fitting`), drawn on the
    /// poles and arms in `props` they belong to.
    pub fittings: Vec<signals::Fitting>,
}

/// The California values the rules read (`caMetric`), named.
#[derive(Clone, Debug)]
pub struct Standards {
    pub sign_mount_min: f64,
    pub sign_mount_secondary_min: f64,
    pub sign_lateral_typical: f64,
    pub sign_lateral_limited: f64,
    pub throughway_min: f64,
    /// How near the kerb face a vertical element may stand (`sidewalk-edge-zone-sf`
    /// `verticalElementOffsetFromCurb`, 18 in): the controller cabinet's back, m.
    pub kerb_vertical_offset: f64,
    pub hydrant_clear: f64,
    pub signal_approach_near_side: f64,
    pub signal_view_min: f64,
    pub detectable_warning_depth: f64,
    pub no_parking_driveway: f64,
    pub no_parking_daylighting: f64,
    /// Street tree basins (`street-tree-basin-sf`, Director's Order 187,246): per sidewalk
    /// class (the surveyed width it starts at, the basin across the walk, along the kerb), m.
    pub tree_basin: [[f64; 3]; 4],
    /// Signal mast arms (`signal-mast-arm-caltrans`, Caltrans RSP ES-7E): one row per arm
    /// length class (`signals::ArmClass`).
    pub signal_arms: Vec<signals::ArmClass>,
    /// Pedestrian push buttons (`ped-push-button-location`, CA MUTCD 4I.05): mounting height
    /// above the sidewalk (m).
    pub ped_button_height: f64,
    /// The push button's R10-3e(CA) sign over it (`ped-push-button-sign`): width, height (m).
    pub ped_button_sign: [f64; 2],
    /// Overhead Street Name signs' upper-case letter height (`street-name-sign-overhead-letter-height`), m.
    pub overhead_name_cap: f64,
    /// Signal faces over the roadway (`signal-mounting-height-over-roadway`): the lowest a
    /// housing's bottom may hang (the binding 15 ft) and the highest its top should (m).
    pub signal_bottom_min: f64,
    pub signal_top_max: f64,
    /// Overhead signs' least vertical clearance over the roadway (`sign-mounting-height-urban`
    /// `overheadMin`, CA MUTCD 2A.15, 17 ft): a Street Name sign under a mast arm, m.
    pub sign_overhead_min: f64,
    /// A signal standard without a luminaire ends this far over its arm's connection
    /// (`signal-mast-pole-top-caltrans`, Caltrans RSP ES-7C Types 16 and 18), m.
    pub mast_pole_top_over_arm: f64,
    /// A mast arm's rise on a San Francisco city street, every street that is not a State
    /// route (`signal-mast-arm-sf-city-street`: level; no SFMTA drawing was found, the
    /// reference photographs show level arms), m. Only level is modelled.
    pub signal_arm_city_rise: f64,
    /// How far a street light may move onto a signal mast pole and stay the same light
    /// (`streetlight-sf-like-for-like-move`, SFPUC Streetlight Guidelines: within 10 ft, the
    /// same fixture, mounting height and orientation), m.
    pub lamp_share_move_max: f64,
}

impl Standards {
    pub fn load(ca: &CaStandards) -> Result<Self> {
        Ok(Self {
            sign_mount_min: ca.metric_key("sign-mounting-height-urban", "min")?,
            sign_mount_secondary_min: ca.metric_key("sign-mounting-height-urban", "secondarySignMin")?,
            sign_lateral_typical: ca.metric_key("sign-lateral-offset-urban", "typicalFromCurbFace")?,
            sign_lateral_limited: ca.metric_key("sign-lateral-offset-urban", "minWhereLimited")?,
            throughway_min: ca.metric_key("sidewalk-throughway-zone-sf", "alleyOrAdaMin")?,
            kerb_vertical_offset: ca.metric_key("sidewalk-edge-zone-sf", "verticalElementOffsetFromCurb")?,
            hydrant_clear: ca.metric("no-parking-fire-hydrant")?,
            signal_approach_near_side: ca.metric_key("no-parking-signalized-approach", "nearSide")?,
            signal_view_min: ca.metric_key("signal-face-longitudinal-position", "min")?,
            detectable_warning_depth: ca.metric_key("detectable-warning-size", "depthMin")?,
            no_parking_driveway: ca.metric("no-parking-driveway")?,
            no_parking_daylighting: ca.metric_key("no-parking-crosswalk-daylighting", "approachSide")?,
            tree_basin: {
                let k = |key: &str| ca.metric_key("street-tree-basin-sf", key);
                [
                    [k("minSidewalkForTrees")?, k("basinAcrossWalk7ft6")?, k("basinAlongWalk7ft6")?],
                    [k("walk11ft")?, k("basinAcrossWalk11")?, k("basinAlongWalk11")?],
                    [k("walk12ft")?, k("basinAcrossWalk12")?, k("basinAlongWalk12")?],
                    [k("walk13ft")?, k("basinAcrossWalk13")?, k("basinAlongWalk13")?],
                ]
            },
            signal_arms: signals::ArmClass::table(ca)?,
            ped_button_height: ca.metric_key("ped-push-button-location", "heightApprox")?,
            ped_button_sign: [ca.metric_key("ped-push-button-sign", "r10_3eWidth")?, ca.metric_key("ped-push-button-sign", "r10_3eHeight")?],
            overhead_name_cap: ca.metric_key("street-name-sign-overhead-letter-height", "upper")?,
            signal_bottom_min: ca.metric_key("signal-mounting-height-over-roadway", "min")?,
            signal_top_max: ca.metric_key("signal-mounting-height-over-roadway", "maxTopGuidance")?,
            sign_overhead_min: ca.metric_key("sign-mounting-height-urban", "overheadMin")?,
            mast_pole_top_over_arm: ca.metric_key("signal-mast-pole-top-caltrans", "poleTopOverArmConnection")?,
            signal_arm_city_rise: {
                let r = ca.metric_key("signal-mast-arm-sf-city-street", "rise")?;
                if r != 0.0 {
                    anyhow::bail!("signal-mast-arm-sf-city-street: a rise of {r} m; only level city arms are modelled");
                }
                r
            },
            lamp_share_move_max: ca.metric_key("streetlight-sf-like-for-like-move", "moveMax")?,
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
    /// The grade-separated roads over the street, when the world has them
    /// (`--structures`; `with_structures`): the props keep clear of them
    /// (`clearance`). None, or an empty world, places what it always did.
    pub structures: Option<Arc<eo_structures::StructureWorld>>,
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
    /// `--tree-basins` (`basins::tree_basins()` when the world was built): a basin under each street tree.
    pub tree_basins: bool,
    /// The street-furniture walk's world (built on first use). // props2
    furniture: std::sync::OnceLock<Arc<furniture::Furniture>>,
    /// With structures: the clearance's sizes, each signalised junction
    /// kept clear, and the deck luminaires by tile (built on first use).
    clear_dims: std::sync::OnceLock<clearance::Dims>,
    clear_nodes: Mutex<HashMap<u32, Arc<signals::ClearedNode>>>,
    deck_lamps: std::sync::OnceLock<HashMap<(i32, i32), Vec<clearance::DeckLamp>>>,
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
            structures: None,
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
            tree_basins: basins::tree_basins(),
            furniture: std::sync::OnceLock::new(),
            clear_dims: std::sync::OnceLock::new(),
            clear_nodes: Mutex::new(HashMap::new()),
            deck_lamps: std::sync::OnceLock::new(),
            build_ms: 0.0,
        };
        w.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        w
    }

    /// The same world over the grade-separated roads `structures` (None
    /// without `--structures`). Set before anything is placed.
    pub fn with_structures(mut self, structures: Option<Arc<eo_structures::StructureWorld>>) -> Self {
        self.structures = structures;
        self
    }

    /// Every street tile key with anything to place.
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        self.street.tile_keys()
    }

    /// The grade-separated roads the props keep clear of: Some only for a
    /// world with something built (`clearance`).
    pub fn clear_structures(&self) -> Option<&eo_structures::StructureWorld> {
        self.structures.as_deref().filter(|s| !s.is_empty())
    }

    /// The sizes the clearance reads (built on first use).
    pub fn clear_dims(&self) -> &clearance::Dims {
        self.clear_dims.get_or_init(|| clearance::Dims::new(self))
    }

    /// The luminaires on the built freeway decks (`--structures`), by the
    /// tile of their foot: the landmark deck rule (`eo_landmarks::lights`:
    /// both edges, every `DECK_PITCH_M` from half a pitch in, the head
    /// `DECK_HEAD_M` over the deck), each a district cobra head on a pole
    /// standing on the barrier top (`StructureWorld::barrier_at`), scaled so
    /// its head is there, with the district lights' own luminaire record
    /// (`Furniture::luminaire`), so it lights through the same lamp list;
    /// none where a barrier is left out (a merge) or another deck is in the
    /// way. Empty without structures.
    /// The lights of the tunnel bores in one tile (stage A7: only with a
    /// structure world built with `--tunnels`; none otherwise), as district
    /// luminaires (`Furniture::luminaire`, the cobra head's optics) hung
    /// under each bore's crown (`StructureWorld::tunnel_lights`), aimed down
    /// at its roadway: what `plan_tile` puts in `TilePlacement::lights` with
    /// the street's own. A host that lights them by day too (a tunnel is lit
    /// round the clock) finds them here.
    pub fn tunnel_lights(&self, key: (i32, i32)) -> Vec<furniture::LampLight> {
        let Some(s) = self.clear_structures() else { return vec![] };
        let f = self.furniture();
        s.tunnel_lights(key).into_iter().map(|l| f.luminaire(l.pos, [l.pos[0], l.floor, l.pos[2]], l.pos[1] - l.floor, Model::LampCobra)).collect()
    }

    pub fn deck_lamps(&self) -> &HashMap<(i32, i32), Vec<clearance::DeckLamp>> {
        self.deck_lamps.get_or_init(|| match self.clear_structures() {
            Some(s) => clearance::place_deck_lamps(self, s),
            None => HashMap::new(),
        })
    }

    /// A signalised junction's signal props as `plan_tile` draws them (kept clear of the
    /// structures where the world has them), before its street lights join its mast poles.
    pub fn signal_node_props(&self, ni: usize) -> Vec<Prop> {
        match self.clear_structures() {
            Some(s) => self.cleared_node(s, ni as u32).props.clone(),
            None => {
                let (mut v, mut f) = (vec![], vec![]);
                signals::plan_node_with(self, ni, &mut v, &mut f);
                v
            }
        }
    }

    /// One signalised junction kept clear of the structures (cached).
    fn cleared_node(&self, s: &eo_structures::StructureWorld, ni: u32) -> Arc<signals::ClearedNode> {
        if let Some(c) = self.clear_nodes.lock().unwrap().get(&ni) {
            return c.clone();
        }
        let c = Arc::new(signals::plan_node_clear(self, s, ni as usize));
        let mut m = self.clear_nodes.lock().unwrap();
        if m.len() > 8192 {
            m.clear();
        }
        m.entry(ni).or_insert(c).clone()
    }

    /// The signal poles the clearance dropped or cut down at the signalised
    /// junctions of a tile and its neighbours (what the overhead hung on
    /// them goes too). Empty without structures.
    pub fn signal_changes_near(&self, key: (i32, i32)) -> Vec<signals::PoleChange> {
        let Some(s) = self.clear_structures() else { return vec![] };
        let mut out = vec![];
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &ni in self.signal_nodes.get(&(key.0 + dx, key.1 + dz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    out.extend(self.cleared_node(s, ni).changes.iter().copied());
                }
            }
        }
        out
    }

    /// One item kept clear of the structures (`clearance::clear_prop`): as
    /// it is, smaller (a tree), or None (dropped); as it is without
    /// structures. For items placed outside this crate (eo-app's park trees).
    pub fn clear_prop(&self, p: Prop) -> Option<Prop> {
        match self.clear_structures() {
            Some(s) => clearance::clear_prop(s, self.clear_dims(), &p, &mut clearance::ClearStats::default()),
            None => Some(p),
        }
    }

    /// Everything placed in one 128 m street tile (owned by position).
    pub fn plan_tile(&self, key: (i32, i32)) -> TilePlacement {
        self.plan_tile_stats(key).0
    }

    /// `plan_tile`, with what keeping clear of the structures did there
    /// (`clearance::ClearStats`; nothing without structures).
    pub fn plan_tile_stats(&self, key: (i32, i32)) -> (TilePlacement, clearance::ClearStats) {
        let clear = self.clear_structures();
        let mut stats = clearance::ClearStats::default();
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
                let first = out.props.len();
                match clear {
                    Some(s) => {
                        let c = self.cleared_node(s, ni);
                        out.props.extend(c.props.iter().copied());
                        out.fittings.extend(c.fittings.iter().cloned());
                        stats.add(&c.stats);
                    }
                    None => signals::plan_node_with(self, ni as usize, &mut out.props, &mut out.fittings),
                }
                // (`--no-signal-luminaires` off) the street lights standing on its mast poles, owned
                // by the junction's tile with the poles: each pole reaches its light's post top.
                for l in signal_lamps::node_lamps(self, ni as usize).iter() {
                    let top = signal_lamps::LAMP_POST_TOP_M * l.prop.scale[1] as f64;
                    if let Some(p) = out.props[first..].iter_mut().find(|p| p.model == Model::SignalPole && p.variant == 1 && p.pos == l.pole) {
                        p.scale[1] = p.scale[1].max(top as f32);
                    }
                    out.props.push(l.prop);
                    out.lights.extend(l.light);
                    out.fittings.push(signals::Fitting::Luminaire { pole: l.pole });
                }
            }
        }
        match clear {
            Some(s) => {
                out.props.extend(signs::plan_tile_clear(self, s, key));
                clearance::clear_tile(self, s, key, &mut out, &mut stats);
            }
            None => out.props.extend(signs::plan_tile(self, key)),
        }
        (out, stats)
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
        let first_prop = ep.props.len();
        ep.props.extend(fe.props.iter().copied());
        // (`--tree-basins`) a basin under each street tree and palm on this ribbon's footways.
        if self.tree_basins {
            basins::entry_basins(self, e, &fe.props, &mut ep.props);
        }
        ep.lights.extend(fe.lights.iter().copied());
        // (eo-app `--no-shared-poles` turns it off) a street light beside a trolley support pole
        // stands on the pole's spot instead, and holds its wires (`overhead::Overhead::lamp_moves`).
        if overhead::lamp_standards() {
            let f = self.furniture();
            for m in overhead::shared(self).lamp_moves(self, ei).iter() {
                let it = &fe.items[m.item];
                let from = [it.x, it.y, it.z];
                let Some((prop, light)) = f.moved_lamp(it, m.to) else { continue };
                if let Some(p) = ep.props[first_prop..].iter_mut().find(|p| p.model == prop.model && p.pos == from) {
                    *p = prop;
                }
                if let (Some(l), Some(nl)) = (ep.lights.iter_mut().find(|l| l.base == from), light) {
                    *l = nl;
                }
            }
        }
        // (`--no-signal-luminaires` off) a street light beside a signal mast pole stands on the
        // pole instead: its junction draws it (`signal_lamps`), this ribbon no longer.
        for m in signal_lamps::entry_moves(self, ei as usize) {
            if let Some(k) = ep.props[first_prop..].iter().position(|p| matches!(p.model, Model::LampCobra | Model::LampBanner) && p.pos == m.from) {
                ep.props.remove(first_prop + k);
            }
            ep.lights.retain(|l| l.base != m.from);
        }
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
