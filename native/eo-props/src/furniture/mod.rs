//! The street-furniture walk of street-furniture.js, ported (CPU only).
//!
//! What it places, and by which JS rule:
//!
//! * corners (`furnishCornerApproach`): every junction corner's forced sign
//!   pole, and a hydrant / bollard by `streetHash32(<seed>:corner:<node>:<segment>:<side>)`;
//! * per ribbon side, in the JS job order: surveyed transit stops and their
//!   shelters (`placeSurveyedTransit`, the bench fallback
//!   `placeSurveyedShelterFallback`), the DataSF street trees
//!   (`placeSurveyedTrees`, with the alley fit `surveyedAlleyTreeFit` and the
//!   overlay's added trees), SFMTA meters and pay stations
//!   (`placeSurveyedMeters`), surveyed hydrants (`placeSurveyedHydrants`), the
//!   block overlays' observed kit (`placeObservedKit`), surveyed kerb dressing
//!   (`placeSurveyedDressing`: benches, bins, post boxes, SFMTA bike racks),
//!   the district street lights (`placeStreetLights`, lamps on: the game loads
//!   the lamp GLB), then the hashed walk (`createFurnitureSideCursor` /
//!   `advanceFurnitureSideCursor` with `chooseKind`);
//! * every placement through `placeItem`: kerb ramp windows, `lateralFor`,
//!   the facade snap for wall kinds (`wallStandOff`), the occupancy grid
//!   (2.5 m cells, `exclusive`), `itemOnRoadway` on the drawn spin, the
//!   inside-building test, and the spin / scale / variant / tree size draws of
//!   `streetRandom(<seed>:item:<segment>:<side>:<round(station*100)>:<kind>)`.
//!
//! The candidate population is the one the JS generates with
//! `generatingCandidates` (every slot is a near-ring slot), which is what the
//! oracle records; the camera rings and their caps are not applied.
//!
//! DIFFERENCES FROM THE JS, STATED. The JS walk is window-global and order
//! dependent: one occupancy grid over the window, all corners first in node
//! order, then every segment in plan order, and a surveyed point is taken by
//! the first segment that places it. Here a 128 m tile must not depend on
//! which tiles were planned before, so:
//!   - a ribbon is planned on its own (cached per ribbon), its occupancy seeded
//!     with the forced corner items of its two end nodes (every approach of
//!     each, planned per node with the node's own occupancy); the walk of a
//!     neighbouring ribbon is not seen (the JS sees whatever ribbon came first);
//!   - a surveyed point (stop, tree, meter, hydrant, dressing) is offered only to
//!     the ribbon that is nearest among those whose own candidate filter takes
//!     it (station in the run, lateral reach, side); the JS offers it to every
//!     segment in order until one places it;
//!   - the legacy scene occupancy (`seedOccupancyFromScene`) is empty, as it is
//!     in the headless oracle; in the browser it holds the renderer's own props;
//!   - the camera rings, ring caps and LOD trims, the fixture dressing
//!     (`furnishFixtures`: off whenever the district lights are on) and the
//!     drawn-paving grounding (the item stands on `PropsWorld::ground`) are not
//!     ported.
//! Segment and node ids seed the streams: the runtime uses the stable key (a
//! JS boot window numbers its ids), tests substitute the JS ids (`SeedIds`).

pub mod data;
pub mod observed;

use crate::{Band, Live, Model, Prop, PropsWorld, StreetRandom, project, station_at};
use data::{FurnitureJs, KindSpec, Overlay, Zone};
use eo_street::hash32;
use eo_street::plan::{Entry, RampWin, segment_kerb_ramps};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub type P2 = [f64; 2];

/// One placed item (the JS `item`, the fields the port keeps).
#[derive(Clone, Debug)]
pub struct Item {
    /// Index into `Furniture::kinds` (catalogue order).
    pub kind: u16,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// three.js `rotation.y`.
    pub rotation: f64,
    pub scale: f64,
    pub height_scale: f64,
    pub variant: u32,
    pub surveyed: bool,
    pub forced: bool,
    pub entry: u32,
    pub side: i8,
    pub station: f64,
    pub node: Option<u32>,
}

/// A street light's luminaire (the JS lamp practical seated on it, at full `lampsOn`).
#[derive(Clone, Copy, Debug)]
pub struct LampLight {
    /// The luminaire (`streetLampHead`): the spot light sits `source_lift_m` above it.
    pub pos: [f64; 3],
    /// The post's foot (the lamp item).
    pub base: [f64; 3],
    /// `LAMP_PRACTICALS.color` (sRGB hex) in linear RGB.
    pub colour_linear: [f32; 3],
    /// Candela at full `lampsOn`: `LAMP_PRACTICALS.intensity` x the pool's nadir gain (`lampPoolOptics`).
    pub intensity: f32,
    /// three.js `SpotLight.distance` (windowed cutoff, metres from the lifted source): `distance` + lift.
    pub range_m: f32,
    /// `decay` (2: inverse square).
    pub decay: f32,
    /// Cone half-angle (rad) and penumbra fraction, aimed straight down.
    pub angle: f32,
    pub penumbra: f32,
    /// The virtual source's lift over the luminaire (`sourceLiftMH` x head height).
    pub source_lift_m: f32,
    pub kind: Model,
}

/// What one ribbon places.
#[derive(Default)]
pub struct EntryFurniture {
    pub items: Vec<Item>,
    pub props: Vec<Prop>,
    pub lights: Vec<LampLight>,
}

/// Seed ids substituted for the runtime's stable keys (tests: a JS boot window's ids).
#[derive(Clone, Debug, Default)]
pub struct SeedIds {
    pub entries: HashMap<u32, String>,
    pub nodes: HashMap<u32, String>,
}

// ---------------------------------------------------------------------------
// indexes
// ---------------------------------------------------------------------------

struct BRec {
    min: P2,
    max: P2,
    poly: Vec<P2>,
    year: Option<f64>,
    use_: Option<u16>,
    retail: bool,
    usage: Option<u16>,
    shop: bool,
}

/// `buildBuildingIndex` (cell 24 m).
struct Buildings {
    recs: Vec<BRec>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    uses: Vec<String>,
    usages: Vec<String>,
}

const B_CELL: f64 = 24.0;

fn point_in_polygon(poly: &[P2], x: f64, z: f64) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        let dz = b[1] - a[1];
        if (a[1] > z) != (b[1] > z) && x < ((b[0] - a[0]) * (z - a[1])) / (if dz == 0.0 { 1e-12 } else { dz }) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn distance_to_polygon(poly: &[P2], x: f64, z: f64) -> f64 {
    let mut best = f64::INFINITY;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        let d = crate::roadway::seg_distance(x, z, a, b);
        if d < best {
            best = d;
        }
        j = i;
    }
    best
}

impl Buildings {
    /// The footprints of the 3 x 3 cells round (x, z), each once, in the JS scan order.
    fn scan(&self, x: f64, z: f64, mut f: impl FnMut(u32, &BRec) -> bool) {
        let (gx, gz) = ((x / B_CELL).floor() as i32, (z / B_CELL).floor() as i32);
        let mut seen: Vec<u32> = vec![];
        for i in -1..=1 {
            for j in -1..=1 {
                let Some(b) = self.cells.get(&(gx + i, gz + j)) else { continue };
                for &id in b {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.push(id);
                    if !f(id, &self.recs[id as usize]) {
                        return;
                    }
                }
            }
        }
    }

    /// `buildingDistanceWithin(index, x, z, cap)`.
    fn distance_within(&self, x: f64, z: f64, cap: f64) -> f64 {
        let limit = (cap + 1e-6) * (cap + 1e-6);
        let mut best = f64::INFINITY;
        let mut inside = false;
        self.scan(x, z, |_, b| {
            let dx = (b.min[0] - x).max(0.0).max(x - b.max[0]);
            let dz = (b.min[1] - z).max(0.0).max(z - b.max[1]);
            let box2 = dx * dx + dz * dz;
            if box2 > limit {
                return true;
            }
            if box2 == 0.0 && point_in_polygon(&b.poly, x, z) {
                inside = true;
                return false;
            }
            if dx.hypot(dz) >= best {
                return true;
            }
            let d = distance_to_polygon(&b.poly, x, z);
            if d < best {
                best = d;
            }
            true
        });
        if inside {
            return 0.0;
        }
        if best <= cap { best } else { f64::INFINITY }
    }

    /// `frontageAt(index, x, z, reach)`: the nearest footprint within reach.
    fn frontage_at(&self, x: f64, z: f64, reach: f64) -> Option<u32> {
        let mut best = reach;
        let mut found = None;
        self.scan(x, z, |id, b| {
            let dx = (b.min[0] - x).max(0.0).max(x - b.max[0]);
            let dz = (b.min[1] - z).max(0.0).max(z - b.max[1]);
            if dx.hypot(dz) >= best {
                return true;
            }
            let d = distance_to_polygon(&b.poly, x, z);
            if d < best {
                best = d;
                found = Some(id);
            }
            true
        });
        found
    }

    /// `frontageYearAt(index, x, z, reach)`.
    fn frontage_year_at(&self, x: f64, z: f64, reach: f64) -> Option<f64> {
        self.frontage_at(x, z, reach).and_then(|id| self.recs[id as usize].year)
    }
}

/// `buildRoadwayIndex(plan)` of street-furniture.js (its own: whole ribbons and the corner pads).
struct Roadway {
    cells: HashMap<(i32, i32), Vec<u32>>,
    pieces: Vec<(P2, P2, f64)>,
    pads: HashMap<(i32, i32), Vec<u32>>,
    rings: Vec<(Vec<P2>, P2, P2)>,
}

const R_CELL: f64 = 24.0;

impl Roadway {
    fn new(w: &PropsWorld) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut pieces = vec![];
        for e in &w.street.plan.entries {
            for i in 0..e.points.len().saturating_sub(1) {
                let (a, b) = (e.points[i], e.points[i + 1]);
                let id = pieces.len() as u32;
                pieces.push((a, b, e.half));
                let reach = e.half + 3.0;
                for gx in ((a[0].min(b[0]) - reach) / R_CELL).floor() as i32..=((a[0].max(b[0]) + reach) / R_CELL).floor() as i32 {
                    for gz in ((a[1].min(b[1]) - reach) / R_CELL).floor() as i32..=((a[1].max(b[1]) + reach) / R_CELL).floor() as i32 {
                        cells.entry((gx, gz)).or_default().push(id);
                    }
                }
            }
        }
        let mut pads: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut rings = vec![];
        for n in &w.street.plan.nodes {
            let ring: Vec<P2> = n.paths.iter().flat_map(|p| p.stations.iter().map(|s| [s.x, s.z])).collect();
            if ring.len() < 3 {
                continue;
            }
            let mut mn = [f64::INFINITY; 2];
            let mut mx = [f64::NEG_INFINITY; 2];
            for p in &ring {
                mn = [mn[0].min(p[0]), mn[1].min(p[1])];
                mx = [mx[0].max(p[0]), mx[1].max(p[1])];
            }
            let id = rings.len() as u32;
            for gx in (mn[0] / R_CELL).floor() as i32..=(mx[0] / R_CELL).floor() as i32 {
                for gz in (mn[1] / R_CELL).floor() as i32..=(mx[1] / R_CELL).floor() as i32 {
                    pads.entry((gx, gz)).or_default().push(id);
                }
            }
            rings.push((ring, mn, mx));
        }
        Roadway { cells, pieces, pads, rings }
    }

    /// `onPavedRoadway(index, x, z, radius)`.
    fn hit(&self, x: f64, z: f64, radius: f64) -> bool {
        let (gx, gz) = ((x / R_CELL).floor() as i32, (z / R_CELL).floor() as i32);
        for i in -1..=1 {
            for j in -1..=1 {
                let key = (gx + i, gz + j);
                if let Some(v) = self.cells.get(&key) {
                    for &id in v {
                        let (a, b, half) = self.pieces[id as usize];
                        if crate::roadway::seg_distance(x, z, a, b) < half + radius {
                            return true;
                        }
                    }
                }
                if let Some(v) = self.pads.get(&key) {
                    for &id in v {
                        let (ring, mn, mx) = &self.rings[id as usize];
                        if x < mn[0] - radius || x > mx[0] + radius || z < mn[1] - radius || z > mx[1] + radius {
                            continue;
                        }
                        if point_in_polygon(ring, x, z) {
                            return true;
                        }
                        let n = ring.len();
                        let mut l = n - 1;
                        for k in 0..n {
                            if crate::roadway::seg_distance(x, z, ring[l], ring[k]) < radius {
                                return true;
                            }
                            l = k;
                        }
                    }
                }
            }
        }
        false
    }
}

/// `makeOccupancy(2.5)`: plan circles.
#[derive(Clone, Default)]
struct Occupancy {
    map: HashMap<(i32, i32), Vec<u32>>,
    circles: Vec<(f64, f64, f64)>,
}

const OCC_CELL: f64 = 2.5;

impl Occupancy {
    fn hit(&self, x: f64, z: f64, r: f64) -> bool {
        let (gx, gz) = ((x / OCC_CELL).floor() as i32, (z / OCC_CELL).floor() as i32);
        let reach = ((r + OCC_CELL) / OCC_CELL).ceil() as i32;
        for i in -reach..=reach {
            for j in -reach..=reach {
                if let Some(b) = self.map.get(&(gx + i, gz + j)) {
                    for &c in b {
                        let (cx, cz, cr) = self.circles[c as usize];
                        let (dx, dz, rr) = (cx - x, cz - z, cr + r);
                        if dx * dx + dz * dz < rr * rr {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn add(&mut self, x: f64, z: f64, r: f64) {
        let id = self.circles.len() as u32;
        self.circles.push((x, z, r));
        let (gx, gz) = ((x / OCC_CELL).floor() as i32, (z / OCC_CELL).floor() as i32);
        self.map.entry((gx, gz)).or_default().push(id);
        let reach = (r / OCC_CELL).ceil() as i32;
        for i in -reach..=reach {
            for j in -reach..=reach {
                if i == 0 && j == 0 {
                    continue;
                }
                self.map.entry((gx + i, gz + j)).or_default().push(id);
            }
        }
    }
}

/// A surveyed point (`city.streetPoints`, the meter list, the bike-rack list).
#[derive(Clone, Copy, Debug)]
struct SPoint {
    x: f64,
    z: f64,
    /// The survey's attribute character (stops), the rack count (SFMTA racks), or the meter kind.
    attr: u32,
    /// The feature kind (dressing: index into `DRESSING`), else 0.
    kind: u8,
}

/// `buildSurveyedIndex` / `meterSurvey`: points filed by 32 m cell, in the JS fill order.
#[derive(Default)]
struct Survey {
    points: Vec<SPoint>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Survey {
    fn push(&mut self, cell: f64, p: SPoint) {
        let id = self.points.len() as u32;
        self.points.push(p);
        self.cells.entry(((p.x / cell).floor() as i32, (p.z / cell).floor() as i32)).or_default().push(id);
    }
}

const DRESSING: [&str; 4] = ["bench", "wasteBasket", "postBox", "bicycleParking"];

/// Survey layers (for the ownership rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Layer {
    Stop,
    Tree,
    Meter,
    Hydrant,
    Dressing,
}

/// The street trees as `surveyedTreeRecord` makes them (with the overlays' reshaping).
struct TreeRec {
    x: f64,
    z: f64,
    form: String,
    crown_m: Option<f64>,
    height_m: Option<f64>,
    leaf: Option<String>,
}

// ---------------------------------------------------------------------------
// the furniture world
// ---------------------------------------------------------------------------

/// The surveyed-first walk (`--surveyed-furniture`; off: the JS walk as ported).
///
/// The JS fills the gaps between surveyed items with a hashed walk that picks
/// kinds by weight, the same on a residential block as on a shopping street:
/// over a citywide sample of 265 tiles it stands 4,486 bollards, 1,588
/// planters, 1,717 bins, 1,305 benches and 1,045 post boxes, and on the
/// citywide audit (eo-app `EO_CAPTURE_STREETS`) a bollard is on 13 of 24
/// streets. The surveys of what stands on SF's kerbs (stops, trees, meters,
/// hydrants, benches, bins, post boxes, bike racks) cover only part of the
/// city, so the walk must still fill in; with this on it fills in what a
/// San Francisco footway has: a slot the walk gives to a kind rare on SF
/// footways (`RARE_KINDS`: bollards, planters, barricades, scaffolding,
/// newsracks) stays empty, and one it gives to a street-commerce kind
/// (`COMMERCE_KINDS`: benches, bins, post boxes, bike racks, café sets,
/// sandwich boards) stays empty unless the frontage there trades (retail, a
/// shop, a ground-floor use, a trade usage). An emptied slot is stepped over
/// as a placed item would be, with the same one draw, so the walk's later
/// draws line up with the JS walk's; where the JS stepped only 1.6 m past a
/// placement that failed, the other hashed kinds thin a little (hydrants by
/// about a sixth on that sample, to about one every 170 m of footway). A
/// corner keeps its forced sign pole and its hashed hydrant but not its
/// hashed bollard. Surveyed and observed items are placed as before.
static SURVEYED_FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Kinds rare on San Francisco footways: a walk slot given to one stays empty under `--surveyed-furniture`.
pub const RARE_KINDS: [&str; 5] = ["bollard", "planter", "barricade", "scaffold", "newsBox"];
/// Street-commerce kinds: a walk slot given to one stays empty away from a trading frontage under `--surveyed-furniture`.
pub const COMMERCE_KINDS: [&str; 6] = ["bench", "wasteBin", "mailbox", "bikeRack", "cafeSet", "sandwichBoard"];

/// Set before the furniture world is built (eo-app, from `--surveyed-furniture`).
pub fn set_surveyed_first(on: bool) {
    SURVEYED_FIRST.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn surveyed_first() -> bool {
    SURVEYED_FIRST.load(std::sync::atomic::Ordering::Relaxed)
}

static POSTER_HOARDINGS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set before the furniture world is built (eo-app, from `--poster-hoardings`). Off (the
/// default): the walk still plans every `posterHoarding` (its slot and its draws stay, so
/// every other item stands where it did) but none is drawn; the flat-toned board stood
/// free of the walls and the slope and read as stray debris along the shopfronts.
pub fn set_poster_hoardings(on: bool) {
    POSTER_HOARDINGS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn poster_hoardings() -> bool {
    POSTER_HOARDINGS.load(std::sync::atomic::Ordering::Relaxed)
}

pub struct Furniture {
    pub js: FurnitureJs,
    pub kinds: Vec<KindSpec>,
    kind_index: HashMap<String, u16>,
    models: Vec<Option<Model>>,
    seed: String,
    roadway: Roadway,
    buildings: Buildings,
    stops: Survey,
    hydrants: Survey,
    dressing: Survey,
    meters: Survey,
    tree_over: HashMap<u32, data::TreeOverride>,
    light_ways: HashMap<i64, i64>,
    dist_ways: HashSet<i64>,
    /// `caMetric('sidewalk-throughway-zone-sf', 'alleyOrAdaMin')`: the lamps' `through`.
    pub lamp_through_m: f64,
    /// Refuse a seat inside a drawn footprint the JS index lacks (on; `EO_FURNITURE_PACK_GUARD=0` turns it off).
    pub guard_pack: bool,
    /// `--surveyed-furniture` (`surveyed_first()` when the world was built): the hashed walk
    /// invents nothing the city surveys and none of the rare kinds (`SURVEYED_FIRST_DROP`).
    pub surveyed_first: bool,
    lamp_colour: [f32; 3],
    node_cache: Mutex<HashMap<u32, Arc<Vec<Item>>>>,
    entry_cache: Mutex<HashMap<u32, Arc<EntryFurniture>>>,
    owners: Mutex<HashMap<(Layer, u32), u32>>,
    pub build_ms: f64,
}

fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 { c * 0.0773993808 } else { (c * 0.9478672986 + 0.0521327014).powf(2.4) }
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// A JS number as `${n}` writes an integer-valued one.
fn int_str(x: f64) -> String {
    format!("{}", x as i64)
}

impl Furniture {
    pub fn new(w: &PropsWorld) -> anyhow::Result<Self> {
        let paths = eo_data::DataPaths::discover()?;
        let city = eo_data::City::load(&paths)?;
        Self::with_buildings(w, FurnitureJs::load()?, &city.buildings)
    }

    pub fn with_buildings(w: &PropsWorld, js: FurnitureJs, buildings: &[eo_data::Building]) -> anyhow::Result<Self> {
        let t0 = std::time::Instant::now();
        let kinds = js.kinds.clone();
        let kind_index: HashMap<String, u16> = kinds.iter().enumerate().map(|(i, k)| (k.id.clone(), i as u16)).collect();
        let models = kinds.iter().map(|k| Model::from_furniture_kind(&k.id)).collect();
        let cell = js.consts.surveyed_cell_m;
        // Stops and hydrants (`buildSurveyedIndex(city, [kind])`).
        let packed = |kind: &str| js.street_points.features.get(kind);
        let mut stops = Survey::default();
        let mut hydrants = Survey::default();
        for (kind, idx) in [("busStop", &mut stops), ("hydrant", &mut hydrants)] {
            if let Some(p) = packed(kind) {
                let attr: Vec<char> = p.attr.chars().collect();
                for i in 0..p.xz.len() / 2 {
                    idx.push(cell, SPoint { x: p.xz[2 * i], z: p.xz[2 * i + 1], attr: *attr.get(i).unwrap_or(&'-') as u32, kind: 0 });
                }
            }
        }
        // Dressing (`surveyedDressingIndex`): OSM benches, bins, post boxes (not
        // OSM bike parking: the SFMTA list replaces it), then the SFMTA racks.
        let mut dressing = Survey::default();
        let sfmta = !js.bike_racks.data.is_empty();
        for (k, kind) in DRESSING.iter().enumerate() {
            if sfmta && *kind == "bicycleParking" {
                continue;
            }
            if !js.consts.surveyed_dressing_kinds.contains_key(*kind) {
                continue;
            }
            if let Some(p) = packed(kind) {
                for i in 0..p.xz.len() / 2 {
                    dressing.push(cell, SPoint { x: p.xz[2 * i], z: p.xz[2 * i + 1], attr: 0, kind: k as u8 });
                }
            }
        }
        if sfmta {
            let d = &js.bike_racks.data;
            for i in 0..d.len() / 3 {
                dressing.push(cell, SPoint { x: d[3 * i] / 10.0, z: d[3 * i + 1] / 10.0, attr: d[3 * i + 2] as u32, kind: 3 });
            }
        }
        let mut meters = Survey::default();
        {
            let d = &js.meters.data;
            for i in 0..d.len() / 3 {
                meters.push(cell, SPoint { x: d[3 * i] / 10.0, z: d[3 * i + 1] / 10.0, attr: d[3 * i + 2] as u32, kind: 0 });
            }
        }
        // Buildings with the frontage attributes, joined by id.
        let mut attrs: HashMap<&str, usize> = HashMap::new();
        for (i, id) in js.buildings.ids.iter().enumerate() {
            attrs.insert(id.as_str(), i);
        }
        let mut recs = vec![];
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut joined = 0usize;
        for b in buildings {
            let poly: Vec<P2> = b.record_polygon().iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
            if poly.len() < 3 {
                continue;
            }
            let mut mn = [f64::INFINITY; 2];
            let mut mx = [f64::NEG_INFINITY; 2];
            for p in &poly {
                mn = [mn[0].min(p[0]), mn[1].min(p[1])];
                mx = [mx[0].max(p[0]), mx[1].max(p[1])];
            }
            let a = attrs.get(b.id.as_str()).or_else(|| attrs.get(format!("sf-building-{}", b.id).as_str())).map(|&i| &js.buildings.attrs[5 * i..5 * i + 5]);
            // The JS loader's buildings only (its window's `city.buildings`: the extract
            // records the export carries); the footprint pack is not in the JS walk's index.
            let Some(a) = a else { continue };
            joined += 1;
            let id = recs.len() as u32;
            recs.push(BRec {
                min: mn,
                max: mx,
                poly,
                year: (a[0] > 1800).then_some(a[0] as f64),
                use_: (a[1] >= 0).then_some(a[1] as u16),
                retail: a[2] != 0,
                usage: (a[3] >= 0).then_some(a[3] as u16),
                shop: a[4] != 0,
            });
            for gx in (mn[0] / B_CELL).floor() as i32..=(mx[0] / B_CELL).floor() as i32 {
                for gz in (mn[1] / B_CELL).floor() as i32..=(mx[1] / B_CELL).floor() as i32 {
                    cells.entry((gx, gz)).or_default().push(id);
                }
            }
        }
        if joined * 2 < js.buildings.ids.len() {
            eprintln!("furniture: only {joined} of the {} JS buildings found in the city", js.buildings.ids.len());
        }
        let bld = Buildings { recs, cells, uses: js.buildings.uses.clone(), usages: js.buildings.usages.clone() };
        let mut light_ways = HashMap::new();
        for c in js.street_lights.ways.chunks_exact(2) {
            light_ways.insert(c[0] as i64, c[1] as i64);
        }
        let dist_ways = js.distribution_ways.iter().map(|v| *v as i64).collect();
        let tree_over = js.tree_overrides.iter().map(|t| (t.i, t.clone())).collect();
        let c = js.consts.lamp_practicals.color;
        let lamp_colour = [((c >> 16) & 255) as f64, ((c >> 8) & 255) as f64, (c & 255) as f64].map(|v| srgb_to_linear(v / 255.0) as f32);
        let mut f = Furniture {
            seed: w.seed.clone(),
            kinds,
            kind_index,
            models,
            roadway: Roadway::new(w),
            buildings: bld,
            stops,
            hydrants,
            dressing,
            meters,
            tree_over,
            light_ways,
            dist_ways,
            lamp_through_m: w.std.throughway_min,
            guard_pack: std::env::var("EO_FURNITURE_PACK_GUARD").map(|v| v != "0").unwrap_or(true),
            surveyed_first: surveyed_first(),
            lamp_colour,
            node_cache: Mutex::new(HashMap::new()),
            entry_cache: Mutex::new(HashMap::new()),
            owners: Mutex::new(HashMap::new()),
            build_ms: 0.0,
            js,
        };
        // The lamps' `through` is the CA throughway minimum (`STREET_LIGHT_THROUGH_M`).
        for k in f.kinds.iter_mut() {
            if matches!(k.id.as_str(), "lampHistoric" | "lampBanner" | "lampCobra") {
                k.through = Some(f.lamp_through_m);
            }
        }
        f.build_ms = t0.elapsed().as_secs_f64() * 1e3;
        Ok(f)
    }

    pub fn kind_id(&self, k: u16) -> &str {
        &self.kinds[k as usize].id
    }

    pub fn kind_of(&self, id: &str) -> Option<u16> {
        self.kind_index.get(id).copied()
    }

    fn k(&self, id: &str) -> usize {
        self.kind_index[id] as usize
    }

    pub fn model_of(&self, k: u16) -> Option<Model> {
        self.models[k as usize]
    }

    /// `furnitureVariantCount(kind, 0)` (the near ring).
    fn variant_count(&self, k: usize) -> usize {
        let kind = &self.kinds[k];
        if kind.id == "tree" {
            return self.js.consts.street_tree_variant_count_near;
        }
        match kind.variants {
            Some(v) if v > 1.0 => v as usize,
            _ => 1,
        }
    }

    fn tree(&self, w: &PropsWorld, i: u32) -> TreeRec {
        let r = crate::trees::record(w, i);
        if let Some(o) = self.tree_over.get(&i) {
            return TreeRec { x: r.x, z: r.z, form: o.form.clone(), crown_m: o.crown_m, height_m: o.height_m, leaf: o.leaf.clone() };
        }
        TreeRec { x: r.x, z: r.z, form: r.form, crown_m: r.crown_m, height_m: r.height_m, leaf: None }
    }

    /// The runtime's seed id of a ribbon: its first piece's stable key.
    fn entry_seed(&self, w: &PropsWorld, ei: usize, ids: Option<&SeedIds>) -> String {
        if let Some(s) = ids.and_then(|m| m.entries.get(&(ei as u32))) {
            return s.clone();
        }
        let e = &w.street.plan.entries[ei];
        e.pieces.first().and_then(|&p| w.street.city.segments.get(p)).and_then(|s| s.key.clone()).unwrap_or_else(|| e.id.clone())
    }

    fn node_seed(&self, w: &PropsWorld, ni: usize, ids: Option<&SeedIds>) -> String {
        if let Some(s) = ids.and_then(|m| m.nodes.get(&(ni as u32))) {
            return s.clone();
        }
        let n = &w.street.plan.nodes[ni];
        n.key.clone().unwrap_or_else(|| n.id.clone())
    }

    /// Is (lat, lon) of (x, z) inside the stop survey's box (`transitSurveyCovers`)?
    fn transit_covers(&self, e: &Entry) -> bool {
        let Some(b) = self.js.street_points.bbox else { return false };
        let f = &self.js.consts.frame;
        !e.points.is_empty()
            && e.points.iter().all(|q| {
                let lat = f.lat + f.z_sign * q[1] / f.meters_per_degree_lat;
                let lon = f.lon + q[0] / f.meters_per_degree_lon;
                lat >= b.min_lat && lat <= b.max_lat && lon >= b.min_lon && lon <= b.max_lon
            })
    }

    /// `distributionWayId(segment)`.
    fn way_id(street_id: Option<&str>) -> Option<i64> {
        street_id?.strip_prefix("sf-street-")?.parse::<i64>().ok()
    }

    /// The surveyed point's owner ribbon: the nearest whose own candidate filter takes it.
    fn owner(&self, w: &PropsWorld, layer: Layer, id: u32, p: P2) -> Option<u32> {
        if let Some(&o) = self.owners.lock().unwrap().get(&(layer, id)) {
            return (o != u32::MAX).then_some(o);
        }
        let cells = crate::signs::entry_cells(w);
        let key = ((p[0] / 32.0).floor() as i32, (p[1] / 32.0).floor() as i32);
        let mut best: Option<(f64, u32)> = None;
        for &ei in cells.get(&key).map(|v| v.as_slice()).unwrap_or(&[]) {
            if let Some(d) = self.takes(w, layer, ei as usize, p) {
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, ei));
                }
            }
        }
        let o = best.map(|b| b.1);
        let mut c = self.owners.lock().unwrap();
        if c.len() > 1 << 21 {
            c.clear();
        }
        c.insert((layer, id), o.unwrap_or(u32::MAX));
        o
    }

    /// Whether a ribbon's candidate filter for `layer` takes the point; its distance.
    fn takes(&self, w: &PropsWorld, layer: Layer, ei: usize, p: P2) -> Option<f64> {
        let e = &w.street.plan.entries[ei];
        let (s0, s1) = (e.trim_start + 1.2, e.length - e.trim_end - 1.2);
        if s1 - s0 < 2.0 {
            return None;
        }
        let (station, distance, lateral) = project(e, p[0], p[1])?;
        let side = if lateral >= 0.0 { 1.0 } else { -1.0 };
        let band = w.band(e, side, self.js.options.through_route)?;
        let seg = w.segment_of(e);
        match layer {
            Layer::Stop => {
                let reach = band.outer + 4.0;
                if station < s0 - 3.0 || station > s1 + 3.0 || lateral.abs() > reach || distance > reach.hypot(3.0) {
                    return None;
                }
            }
            Layer::Tree | Layer::Meter | Layer::Hydrant | Layer::Dressing => {
                let reach = band.outer + if matches!(layer, Layer::Tree | Layer::Meter) { 1.5 } else { 2.0 };
                if layer == Layer::Tree && !tree_covers(w, e) {
                    return None;
                }
                if layer == Layer::Meter && !survey_covers(seg.street_id.as_deref(), e, self.js.meters.bounds) {
                    return None;
                }
                if station < s0 || station > s1 || lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                    return None;
                }
            }
        }
        Some(distance)
    }

    // -----------------------------------------------------------------------
    // caches and the public surface
    // -----------------------------------------------------------------------

    /// What one ribbon places (cached).
    pub fn entry(&self, w: &PropsWorld, ei: usize) -> Arc<EntryFurniture> {
        if let Some(e) = self.entry_cache.lock().unwrap().get(&(ei as u32)) {
            return e.clone();
        }
        let ef = Arc::new(self.plan_entry(w, ei, None));
        let mut c = self.entry_cache.lock().unwrap();
        if c.len() > 8192 {
            c.clear();
        }
        c.insert(ei as u32, ef.clone());
        ef
    }

    /// The forced corner items of one node (cached for the runtime ids).
    fn node_items(&self, w: &PropsWorld, ni: usize, ids: Option<&SeedIds>) -> Arc<Vec<Item>> {
        if ids.is_none() {
            if let Some(v) = self.node_cache.lock().unwrap().get(&(ni as u32)) {
                return v.clone();
            }
        }
        let mut walk = Walk::new(w, self, ids, ni as u32);
        let node = &w.street.plan.nodes[ni];
        for ai in 0..node.approaches.len() {
            walk.corner_approach(ni, ai);
        }
        let v = Arc::new(walk.items);
        if ids.is_none() {
            let mut c = self.node_cache.lock().unwrap();
            if c.len() > 8192 {
                c.clear();
            }
            c.insert(ni as u32, v.clone());
        }
        v
    }

    /// Plan one ribbon: its end nodes' corners, then each side in the JS job order.
    pub fn plan_entry(&self, w: &PropsWorld, ei: usize, ids: Option<&SeedIds>) -> EntryFurniture {
        let e = &w.street.plan.entries[ei];
        let mut walk = Walk::new(w, self, ids, ei as u32);
        let mut ends: Vec<usize> = [e.approach_start, e.approach_end].iter().flatten().map(|a| a.0).collect();
        ends.dedup();
        let mut own: Vec<Item> = vec![];
        for &ni in &ends {
            for it in self.node_items(w, ni, ids).iter() {
                walk.occ.add(it.x, it.z, self.occ_radius(it.kind as usize));
                if it.entry == ei as u32 && !own.iter().any(|o| o.node == it.node && o.kind == it.kind && o.station == it.station && o.side == it.side) {
                    own.push(it.clone());
                }
            }
        }
        for side in [1.0, -1.0] {
            walk.furnish_side(ei, side);
        }
        let mut items = own;
        items.append(&mut walk.items);
        self.finish(w, items)
    }

    fn occ_radius(&self, k: usize) -> f64 {
        let kind = &self.kinds[k];
        kind.exclusive.unwrap_or(kind.radius)
    }

    fn finish(&self, w: &PropsWorld, items: Vec<Item>) -> EntryFurniture {
        let mut props = vec![];
        let mut lights = vec![];
        let hoardings = poster_hoardings();
        for it in &items {
            let Some(model) = self.model_of(it.kind) else { continue };
            if model == Model::PosterHoarding && !hoardings {
                continue;
            }
            props.push(Prop {
                model,
                pos: [it.x, it.y, it.z],
                yaw: it.rotation,
                scale: [it.scale as f32, it.height_scale as f32, it.scale as f32],
                variant: it.variant as u16,
                live: Live::None,
            });
            if let Some(l) = self.lamp_light(it, model) {
                lights.push(l);
            }
        }
        let _ = w;
        EntryFurniture { items, props, lights }
    }

    /// `districtLightFixtures` + `streetLampHead` + `lampPoolOptics` for a district light.
    fn lamp_light(&self, it: &Item, model: Model) -> Option<LampLight> {
        let id = self.kind_id(it.kind);
        if !matches!(id, "lampCobra" | "lampBanner" | "lampHistoric") {
            return None;
        }
        let historic = id == "lampHistoric";
        let heads = &self.js.consts.lamp_head;
        // A historic post has no optics override and is not `arterial`: the column head, no arm.
        let spec = if historic { heads["column"] } else { heads["district"] };
        let toward = [-it.rotation.sin(), -it.rotation.cos()];
        let reach = if historic { 0.0 } else { spec.reach_m };
        let head = [it.x + toward[0] * reach, it.y + spec.height_m, it.z + toward[1] * reach];
        Some(self.luminaire(head, [it.x, it.y, it.z], spec.height_m, model))
    }

    /// A street luminaire's light (`LAMP_PRACTICALS` with `lampPoolOptics`):
    /// its head at `head`, `h` metres over the surface it lights, on the
    /// post whose foot is `base`. The district lights' own, and the deck
    /// luminaires' (`clearance`, stage A6).
    pub fn luminaire(&self, head: [f64; 3], base: [f64; 3], h: f64, model: Model) -> LampLight {
        let p = &self.js.consts.lamp_practicals;
        let (lift, angle, penumbra, gain) = if !(h > 1.0) {
            (0.0, p.angle, p.penumbra, 1.0)
        } else {
            let lift = p.source_lift_m_h * h;
            let big = h + lift;
            let outer = (p.pool_outer_m_h * h / big).atan();
            let inner = (p.pool_inner_m_h * h / big).atan();
            (lift, outer, 1.0 - inner / outer, (big / h).powi(2))
        };
        LampLight {
            pos: head,
            base,
            colour_linear: self.lamp_colour,
            intensity: (p.intensity * gain) as f32,
            range_m: (p.distance + lift) as f32,
            decay: p.decay as f32,
            angle: angle as f32,
            penumbra: penumbra as f32,
            source_lift_m: lift as f32,
            kind: model,
        }
    }
}

/// `treeSurveyCovers(segment)`.
fn tree_covers(w: &PropsWorld, e: &Entry) -> bool {
    let seg = w.segment_of(e);
    if !seg.street_id.as_deref().unwrap_or("").starts_with("sf-street-") {
        return false;
    }
    let b = w.js.trees.bounds;
    e.points.iter().all(|q| q[0] >= b.min_x && q[0] <= b.max_x && q[1] >= b.min_z && q[1] <= b.max_z)
}

/// `surveyCovers(segment, bounds)`.
fn survey_covers(street_id: Option<&str>, e: &Entry, b: data::XzBounds) -> bool {
    if !street_id.unwrap_or("").starts_with("sf-street-") {
        return false;
    }
    e.points.iter().all(|q| q[0] >= b.min_x && q[0] <= b.max_x && q[1] >= b.min_z && q[1] <= b.max_z)
}

/// `classBand(classRank)`.
fn class_band(rank: i32) -> usize {
    if rank <= 2 {
        0
    } else if rank == 3 {
        1
    } else if rank == 4 {
        2
    } else {
        3
    }
}

// ---------------------------------------------------------------------------
// the walk
// ---------------------------------------------------------------------------

/// One side of one ribbon as the walk sees it.
struct SideCtx {
    ei: usize,
    seg_id: String,
    side: f64,
    band: Band,
    s0: f64,
    s1: f64,
    /// The drawn kerb ramp windows of this side.
    ramps: Vec<RampWin>,
}

#[derive(Default, Clone)]
struct TreeSpec {
    variant: Option<usize>,
    form: Option<String>,
    leaf: Option<String>,
    crown_m: Option<f64>,
    height_m: Option<f64>,
}

struct Spec<'k> {
    station: f64,
    kind: usize,
    /// `treeFit.kind`, when the alley fit replaces the kind's footprint.
    fit: Option<&'k KindSpec>,
    forced: bool,
    node: Option<u32>,
    tree: Option<TreeSpec>,
}

struct Walk<'a> {
    w: &'a PropsWorld,
    f: &'a Furniture,
    ids: Option<&'a SeedIds>,
    /// The ribbon (or node) this walk plans.
    owner: u32,
    occ: Occupancy,
    items: Vec<Item>,
    last_rejection: &'static str,
    /// Points this walk has placed (`surveyedClaimed`, per walk).
    claimed: HashSet<(Layer, u32)>,
}

impl<'a> Walk<'a> {
    fn new(w: &'a PropsWorld, f: &'a Furniture, ids: Option<&'a SeedIds>, owner: u32) -> Self {
        Walk { w, f, ids, owner, occ: Occupancy::default(), items: vec![], last_rejection: "", claimed: HashSet::new() }
    }

    fn reject(&mut self, why: &'static str) {
        self.last_rejection = why;
    }

    fn side_ctx(&self, ei: usize, side: f64) -> Option<SideCtx> {
        let e = &self.w.street.plan.entries[ei];
        let band = self.w.band(e, side, self.f.js.options.through_route)?;
        let ramps = segment_kerb_ramps(e, &self.w.street.o);
        Some(SideCtx {
            ei,
            seg_id: self.f.entry_seed(self.w, ei, self.ids),
            side,
            band,
            s0: e.trim_start + 1.2,
            s1: e.length - e.trim_end - 1.2,
            ramps: ramps[if side > 0.0 { 0 } else { 1 }].clone(),
        })
    }

    fn ramp_span(&self, win: &RampWin) -> (f64, f64) {
        let clear = self.f.js.options.kerb_ramp_clear_m;
        let lo = (if win.flare_lo.is_finite() { win.flare_lo.min(win.lo) } else { win.lo }) - clear;
        let hi = (if win.flare_hi.is_finite() { win.flare_hi.max(win.hi) } else { win.hi }) + clear;
        (lo, hi)
    }

    /// `kerbRampBlocker(...)(s)`.
    fn ramp_blocked(&self, sc: &SideCtx, s: f64) -> bool {
        sc.ramps.iter().any(|w| {
            let (lo, hi) = self.ramp_span(w);
            s > lo && s < hi
        })
    }

    /// `kerbRampReservedMetres`.
    fn ramp_reserved(&self, sc: &SideCtx, from: f64, to: f64) -> f64 {
        if sc.ramps.is_empty() || !(to > from) {
            return 0.0;
        }
        let mut spent = 0.0;
        for w in &sc.ramps {
            let (lo, hi) = self.ramp_span(w);
            spent += (hi.min(to) - lo.max(from)).max(0.0);
        }
        spent.min(to - from)
    }

    /// `pastKerbRamp(state, segment, side, s, limit, dir)`.
    fn past_ramp(&self, sc: &SideCtx, s: f64, limit: f64, dir: f64) -> Option<f64> {
        if sc.ramps.is_empty() {
            return Some(s);
        }
        let mut out = s;
        let mut i = 0;
        while i < sc.ramps.len() + 1 && self.ramp_blocked(sc, out) {
            let mut next = out;
            for w in &sc.ramps {
                let (lo, hi) = self.ramp_span(w);
                if !(out > lo && out < hi) {
                    continue;
                }
                next = if dir >= 0.0 { next.max(hi + 0.02) } else { next.min(lo - 0.02) };
            }
            if next == out {
                break;
            }
            out = next;
            i += 1;
        }
        if self.ramp_blocked(sc, out) {
            return None;
        }
        if dir >= 0.0 { (out <= limit).then_some(out) } else { (out >= limit).then_some(out) }
    }

    /// `lateralFor(kind, band, throughRoute)`: (lateral, limit).
    fn lateral_for(&self, kind: &KindSpec, band: &Band) -> (f64, f64) {
        let through = kind.through.unwrap_or(self.f.js.options.through_route);
        let limit = band.outer - through;
        match kind.zone {
            Zone::Building => (band.outer + kind.reach.unwrap_or(self.f.js.options.wall_reach) - kind.depth, band.outer),
            Zone::Mid => {
                let wanted = band.inner + (kind.depth + 0.1).max(band.usable * 0.5);
                (wanted.min(limit - kind.depth), limit)
            }
            Zone::Curb => (band.inner + kind.depth + 0.06, limit),
        }
    }

    /// `itemOnRoadway(state, kind, x, z, ox, oz)`.
    fn on_roadway(&self, kind: &KindSpec, x: f64, z: f64, ox: f64, oz: f64) -> bool {
        let depth = kind.depth.max(0.18) + 0.06;
        let half = (kind.radius - kind.depth).max(0.0);
        let (ax, az) = (-oz, ox);
        let ts: &[f64] = if half > 0.05 { &[-half, 0.0, half] } else { &[0.0] };
        ts.iter().any(|&t| self.f.roadway.hit(x + ax * t, z + az * t, depth))
    }

    /// `placeItem(state, spec)`.
    fn place(&mut self, sc: &SideCtx, spec: Spec) -> Option<usize> {
        let f = self.f;
        let w = self.w;
        let e = &w.street.plan.entries[sc.ei];
        let kind_id = f.kinds[spec.kind].id.as_str();
        let kind: &KindSpec = spec.fit.unwrap_or(&f.kinds[spec.kind]);
        let band = &sc.band;
        let side = sc.side;
        let station = spec.station;
        if kind.zone != Zone::Building && self.ramp_blocked(sc, station) {
            self.reject("in-kerb-ramp");
            return None;
        }
        let wall_reach = f.js.options.wall_reach;
        let reach_kind = KindSpec { reach: Some(wall_reach), ..kind.clone() };
        let (lateral, limit) = self.lateral_for(&reach_kind, band);
        if lateral - kind.depth < band.inner - 0.05 {
            self.reject("inside-curb");
            return None;
        }
        if kind.zone != Zone::Building {
            if lateral + kind.depth > band.outer + 0.05 {
                self.reject("past-property-line");
                return None;
            }
            if lateral + kind.depth > limit + 1e-6 {
                self.reject("blocks-through-route");
                return None;
            }
        }
        let st = station_at(e, station, false);
        let (nx, nz) = (st.nx * st.miter, st.nz * st.miter);
        let mut placed_lateral = lateral;
        if kind.zone == Zone::Building {
            let floor = band.inner + kind.depth + 0.05;
            let ceiling = band.outer + wall_reach;
            let mut snapped = None;
            let mut probe = lateral.min(ceiling);
            while probe >= floor {
                let (px, pz) = (st.x + nx * side * probe, st.z + nz * side * probe);
                let gap = f.buildings.distance_within(px, pz, 0.55);
                if gap > 0.02 && gap <= 0.55 {
                    snapped = Some(probe);
                    break;
                }
                if gap > 0.55 {
                    break;
                }
                probe -= 0.12;
            }
            let Some(s) = snapped else {
                self.reject("no-facade");
                return None;
            };
            placed_lateral = s;
            if let Some(off) = kind.wall_stand_off {
                let gap_now = f.buildings.distance_within(st.x + nx * side * placed_lateral, st.z + nz * side * placed_lateral, off);
                placed_lateral -= (off - gap_now).max(0.0) / nx.hypot(nz).max(1.0);
                if placed_lateral - kind.depth < band.inner + 0.3 {
                    self.reject("stand-off-too-narrow");
                    return None;
                }
            }
        }
        let x = st.x + nx * side * placed_lateral;
        let z = st.z + nz * side * placed_lateral;
        let occ_r = kind.exclusive.unwrap_or(kind.radius);
        if self.occ.hit(x, z, occ_r) {
            self.reject("overlap");
            return None;
        }
        let mut rng = StreetRandom::new(&format!("{}:item:{}:{}:{}:{}", f.seed, sc.seg_id, int_str(side), int_str(js_round(station * 100.0)), kind_id));
        let facing = (side * nx).atan2(side * nz);
        let spin = match kind_id {
            "tree" | "hydrant" | "bollard" | "wasteBin" => (rng.next() - 0.5) * std::f64::consts::PI,
            "barricade" | "sandwichBoard" => (rng.next() - 0.5) * 0.5,
            _ => (rng.next() - 0.5) * 0.06,
        };
        let rotation = facing + spin;
        if self.on_roadway(kind, x, z, rotation.sin(), rotation.cos()) {
            self.reject("on-carriageway");
            return None;
        }
        let clearance = if kind.zone == Zone::Building { 0.02 } else { 0.15 };
        if f.buildings.distance_within(x, z, clearance) <= clearance {
            self.reject("inside-building");
            return None;
        }
        // Not in the JS: a drawn footprint the JS index lacks (the footprint pack) is
        // stood clear of too, so nothing is placed inside a building this world draws.
        if f.guard_pack && w.street.buildings.contains(x, z) {
            self.reject("inside-building");
            return None;
        }
        let o = &w.street.o;
        let surface_lateral = placed_lateral.min(band.outer);
        if !(o.sidewalk_surface_y(0.0, surface_lateral, e.half) >= o.sidewalk_surface_y(0.0, e.half, e.half) - 1e-6) {
            self.reject("below-curb");
            return None;
        }
        let y = w.ground(x, z);
        let tree = kind_id == "tree";
        let mut scale = if tree { 0.82 + rng.next() * 0.5 } else { 0.965 + rng.next() * 0.07 };
        let variants = f.variant_count(spec.kind);
        let ts = spec.tree.clone().unwrap_or_default();
        let variant: usize = if tree {
            if let Some(v) = ts.variant {
                rng.next();
                v
            } else if let Some(form) = ts.form.as_deref() {
                rng.next();
                crate::eztree::surveyed_tree_variant(form, variants, station, ts.leaf.as_deref()).unwrap_or(0)
            } else {
                let year = f.buildings.frontage_year_at(x, z, 12.0);
                crate::eztree::street_tree_variant_for(rng.next(), variants, Some(band.walk), year, Some(e.half)).unwrap_or(0)
            }
        } else if variants > 1 {
            (hash32(&format!("{}:variant:{}:{}:{}", f.seed, sc.seg_id, int_str(side), kind_id)) % variants as u32) as usize
        } else {
            0
        };
        let species = if tree {
            crate::eztree::street_tree_species().ok().and_then(|s| s.get(variant)).and_then(|id| crate::eztree::ez_species(id).ok())
        } else {
            None
        };
        if tree && ts.crown_m.is_some_and(|c| c > 0.0) {
            let half = species.map(|s| s.max_half_crown_m).filter(|v| *v != 0.0 && v.is_finite()).unwrap_or(f.js.consts.street_tree_max_half_crown);
            let factor = species.and_then(|s| s.crown_factor).filter(|v| *v != 0.0).unwrap_or(1.0);
            scale = clamp(ts.crown_m.unwrap() * factor / (2.0 * half), 0.82, 1.35);
        }
        let mut height_scale = if tree { scale * (0.92 + rng.next() * 0.2) } else { scale };
        if tree && ts.height_m.is_some_and(|h| h > 0.0) {
            if let Some(sp) = species {
                let species_height = sp.height_m;
                if species_height > 0.0 {
                    let clear = sp.clearance_m.filter(|v| *v != 0.0).unwrap_or(2.8);
                    let floor = (0.82f64).max(f.js.consts.street_tree_min_leaf_clearance_m / clear);
                    let max_h = sp.max_height_m.filter(|v| *v != 0.0).unwrap_or(f64::INFINITY);
                    let ceiling = floor.max((1.4f64).min(max_h / species_height));
                    let hf = sp.height_factor.filter(|v| *v != 0.0).unwrap_or(1.0);
                    height_scale = clamp((ts.height_m.unwrap() * hf / species_height) * (0.95 + rng.next() * 0.1), floor, ceiling);
                }
            }
        }
        if kind_id == "palm" && ts.height_m.is_some_and(|h| h > 0.0) {
            let reference = f.js.consts.palm_height_m.filter(|v| *v != 0.0).unwrap_or(kind.height);
            scale = clamp(ts.height_m.unwrap() / reference, 0.6, 1.5);
            height_scale = scale;
        }
        let leans = tree && species.and_then(|s| s.street_lean_m).is_some_and(|l| l > 0.0);
        let item_rotation = if leans { facing + spin * 0.1 } else { rotation };
        if item_rotation != rotation && self.on_roadway(kind, x, z, item_rotation.sin(), item_rotation.cos()) {
            self.reject("on-carriageway");
            return None;
        }
        self.occ.add(x, z, occ_r);
        self.items.push(Item {
            kind: spec.kind as u16,
            x,
            y,
            z,
            rotation: item_rotation,
            scale,
            height_scale,
            variant: variant as u32,
            surveyed: false,
            forced: spec.forced,
            entry: sc.ei as u32,
            side: side as i8,
            station,
            node: if spec.forced { spec.node } else { None },
        });
        Some(self.items.len() - 1)
    }

    fn spec(&self, station: f64, kind: usize) -> Spec<'a> {
        Spec { station, kind, fit: None, forced: false, node: None, tree: None }
    }

    // -----------------------------------------------------------------------
    // corners
    // -----------------------------------------------------------------------

    /// `furnishCornerApproach(state, node, approach)`.
    fn corner_approach(&mut self, ni: usize, ai: usize) {
        let w = self.w;
        let node = &w.street.plan.nodes[ni];
        let app = &node.approaches[ai];
        let ei = app.entry;
        let e = &w.street.plan.entries[ei];
        let side = if app.at_start { 1.0 } else { -1.0 };
        let Some(sc) = self.side_ctx(ei, side) else { return };
        let hash = hash32(&format!("{}:corner:{}:{}:{}", self.f.seed, self.f.node_seed(w, ni, self.ids), sc.seg_id, int_str(side)));
        let mut wanted = vec!["signPole"];
        if hash % 5 == 0 {
            wanted.push("hydrant");
        }
        // (`--surveyed-furniture`) the corner's hashed bollard: none.
        if hash % 3 == 0 && !self.f.surveyed_first {
            wanted.push("bollard");
        }
        let dir = if app.at_start { 1.0 } else { -1.0 };
        let limit = if app.at_start { e.length - 0.3 } else { 0.3 };
        let mut offset = 1.3;
        for id in wanted {
            let k = self.f.k(id);
            let kind = &self.f.kinds[k];
            if sc.band.walk < kind.min_walk {
                continue;
            }
            let mut station = if app.at_start { app.trim + offset } else { e.length - app.trim - offset };
            if self.ramp_blocked(&sc, station) {
                let Some(moved) = self.past_ramp(&sc, station, limit, dir) else { continue };
                offset += (moved - station).abs();
                station = moved;
            }
            if station < 0.3 || station > e.length - 0.3 {
                continue;
            }
            let r2 = kind.radius * 2.0 + 0.9;
            self.place(&sc, Spec { forced: true, node: Some(ni as u32), ..self.spec(station, k) });
            offset += r2;
        }
    }

    // -----------------------------------------------------------------------
    // the side
    // -----------------------------------------------------------------------

    fn mark_surveyed(&mut self, idx: Option<usize>) {
        if let Some(i) = idx {
            self.items[i].surveyed = true;
        }
    }

    fn owns(&self, layer: Layer, id: u32, p: P2) -> bool {
        self.f.owner(self.w, layer, id, p) == Some(self.owner)
    }

    /// `surveyedTreesAlong(index, segment, reach)` over a survey (or the tree list).
    fn along(&self, cells: &HashMap<(i32, i32), Vec<u32>>, e: &Entry, reach: f64) -> Vec<u32> {
        let cell = self.f.js.consts.surveyed_cell_m;
        let span = (reach / cell).ceil() as i32;
        let step = cell / 2.0;
        let mut seen = HashSet::new();
        let mut out = vec![];
        let mut s = 0.0;
        while s <= e.length + 1e-6 {
            let st = station_at(e, s.min(e.length), false);
            let (cx, cz) = ((st.x / cell).floor() as i32, (st.z / cell).floor() as i32);
            for a in -span..=span {
                for b in -span..=span {
                    if seen.insert((cx + a, cz + b)) {
                        if let Some(v) = cells.get(&(cx + a, cz + b)) {
                            out.extend_from_slice(v);
                        }
                    }
                }
            }
            s += step;
        }
        out
    }

    /// `createFurnitureSideCursor` + `advanceFurnitureSideCursor`.
    fn furnish_side(&mut self, ei: usize, side: f64) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[ei];
        let Some(sc) = self.side_ctx(ei, side) else { return };
        if sc.s1 - sc.s0 < 2.0 {
            return;
        }
        let seg = w.segment_of(e);
        let overlay = observed::overlay_for(&f.js.overlays, seg.street_id.as_deref());
        let mut rng = StreetRandom::new(&format!("{}:furnish:{}:{}", f.seed, sc.seg_id, int_str(side)));
        let mut suppress: HashSet<&'static str> = HashSet::new();
        self.transit(&sc, overlay, &mut suppress);
        self.trees(&sc, overlay, &mut suppress);
        self.meters(&sc, overlay, &mut suppress);
        self.hydrants(&sc, &mut suppress);
        self.observed_kit(&sc, overlay, &mut suppress);
        self.dressing(&sc, overlay, &mut suppress);
        self.street_lights(&sc);
        // `state.signInventory`: San Francisco's signs are SFMTA's inventory.
        if !w.js.signs.posts.is_empty() {
            suppress.insert("signPole");
        }
        let absent = observed::absent_windows(overlay, e, side);
        let opts = &f.js.options;
        let corner_zone = opts.corner_zone;
        let base_pitch = if e.class_rank >= 5 { 3.0 } else if e.class_rank >= 4 { 3.6 } else { 4.8 };
        let node_start = e.approach_start.map(|a| a.0);
        let node_end = e.approach_end.map(|a| a.0);
        let pitch_factor = |me: &Self, from: f64, to: f64| -> f64 {
            let zone = to - from;
            if !(zone > 0.1) {
                return 1.0;
            }
            clamp((zone - me.ramp_reserved(&sc, from, to)) / zone, 0.3, 1.0)
        };
        let start_pitch = if node_start.is_some() { pitch_factor(self, sc.s0, sc.s1.min(sc.s0 + corner_zone)) } else { 1.0 };
        let end_pitch = if node_end.is_some() { pitch_factor(self, (sc.s1 - corner_zone).max(sc.s0), sc.s1) } else { 1.0 };
        let mut last_at: HashMap<usize, f64> = HashMap::new();
        let mut last_placed: Option<usize> = None;
        let parks = seg.parking[if side > 0.0 { 0 } else { 1 }] > 0.0;
        let pitch_scale = f.js.consts.near_ring["pitchScale"].as_f64().unwrap_or(1.0);
        let mut station = sc.s0 + rng.next() * 1.5;
        let mut guard = 0;
        let band = sc.band;
        while station < sc.s1 && guard < 4000 {
            guard += 1;
            if self.ramp_blocked(&sc, station) {
                match self.past_ramp(&sc, station, sc.s1, 1.0) {
                    Some(n) => {
                        station = n;
                        continue;
                    }
                    None => break,
                }
            }
            let st = station_at(e, station, false);
            let (nx, nz) = (st.nx * st.miter, st.nz * st.miter);
            let at_start = node_start.is_some() && station - sc.s0 < corner_zone;
            let at_end = node_end.is_some() && sc.s1 - station < corner_zone;
            let at_corner = at_start || at_end;
            let corner_node = if at_start { node_start } else if at_end { node_end } else { None };
            let signal_corner = corner_node.is_some_and(|n| w.street.plan.nodes[n].signalised());
            let (ox, oz) = (st.x + nx * side * band.outer, st.z + nz * side * band.outer);
            let gap = f.buildings.distance_within(ox, oz, opts.wall_reach);
            let frontage = if gap <= opts.wall_reach { f.buildings.frontage_at(ox, oz, opts.wall_reach + 0.5) } else { None };
            let along = |a: f64| f.buildings.frontage_at(ox - nz * a, oz + nx * a, opts.wall_reach + 0.5);
            let choice = self.choose_kind(&ChooseCtx {
                band: &band,
                class_rank: e.class_rank,
                at_corner,
                signal_corner,
                last_at: &last_at,
                station,
                building_gap: gap,
                frontage,
                suppress: &suppress,
                parks,
                absent: absent.as_deref(),
                last_placed,
                along: &along,
            }, rng.next());
            let corner_pitch = if at_start { start_pitch } else if at_end { end_pitch } else { 1.0 };
            let Some(k) = choice else {
                station += base_pitch * corner_pitch;
                continue;
            };
            let kr = f.kinds[k].radius;
            // (`--surveyed-furniture`) a rare kind's slot, and a street-commerce kind's away from a
            // trading frontage, stay empty: the walk steps on as if the item stood there, with the
            // same one draw a placement spends, so its later draws (every other item) are as before.
            if f.surveyed_first && self.sf_drops(k, frontage) {
                let step = (kr * 2.0 + 0.5).max(0.9) + base_pitch * pitch_scale * (if at_corner { 0.75 } else { 1.0 }) * (0.6 + rng.next() * 0.7);
                station += if at_corner { (kr * 2.0 + 0.35).max(step * corner_pitch) } else { step };
                continue;
            }
            if self.place(&sc, self.spec(station, k)).is_some() {
                last_at.insert(k, station);
                last_placed = Some(k);
                let step = (kr * 2.0 + 0.5).max(0.9) + base_pitch * pitch_scale * (if at_corner { 0.75 } else { 1.0 }) * (0.6 + rng.next() * 0.7);
                station += if at_corner { (kr * 2.0 + 0.35).max(step * corner_pitch) } else { step };
            } else {
                station += 1.6 + rng.next();
            }
        }
    }

    /// (`--surveyed-furniture`) Whether the walk's choice of kind `k` with this frontage is left empty.
    fn sf_drops(&self, k: usize, frontage: Option<u32>) -> bool {
        let f = self.f;
        let id = f.kinds[k].id.as_str();
        if RARE_KINDS.contains(&id) {
            return true;
        }
        if !COMMERCE_KINDS.contains(&id) {
            return false;
        }
        let b = &f.buildings;
        let trades = frontage.map(|i| &b.recs[i as usize]).is_some_and(|r| {
            r.retail || r.shop || r.use_.is_some() || r.usage.is_some_and(|u| f.js.consts.frontage_trade_usages.iter().any(|x| x == &b.usages[u as usize]))
        });
        !trades
    }

    /// `chooseKind(state, context, draw)`.
    fn choose_kind(&self, c: &ChooseCtx, draw: f64) -> Option<usize> {
        let f = self.f;
        let b = &f.buildings;
        let mut total = 0.0;
        let mut legal: Vec<(usize, f64)> = vec![];
        let fr = c.frontage.map(|i| &b.recs[i as usize]);
        let use_of = |r: &BRec| r.use_.map(|u| b.uses[u as usize].as_str());
        let usage_of = |r: &BRec| r.usage.map(|u| b.usages[u as usize].as_str());
        for (k, kind) in f.kinds.iter().enumerate() {
            let id = kind.id.as_str();
            if c.suppress.contains(id) {
                continue;
            }
            if c.absent.is_some_and(|a| observed::absent_at(a, id, c.station)) {
                continue;
            }
            if kind.catalogue_available == Some(false) || kind.fixture {
                continue;
            }
            if kind.parking_only && !c.parks {
                continue;
            }
            if id == "signPole" && c.at_corner {
                continue;
            }
            if (id == "pedSignalPost" || id == "signalCabinet") && !c.signal_corner {
                continue;
            }
            if c.band.walk < kind.min_walk {
                continue;
            }
            if kind.zone == Zone::Mid && c.band.usable - kind.depth * 2.0 < 1.0 {
                continue;
            }
            if kind.zone == Zone::Building && !(c.building_gap <= f.js.options.wall_reach) {
                continue;
            }
            if let (Some(deny), Some(u)) = (&kind.frontage_deny, fr.and_then(use_of)) {
                if deny.iter().any(|d| d == u) {
                    continue;
                }
            }
            if kind.retail_frontage == Some(false) && fr.is_some_and(|r| r.retail) {
                continue;
            }
            if kind.frontage_evidence {
                let ok = fr.is_some_and(|r| r.use_.is_some() || usage_of(r).is_some_and(|u| f.js.consts.frontage_evidence_usages.iter().any(|x| x == u)));
                if !ok {
                    continue;
                }
            }
            if kind.frontage_trade {
                let ok = fr.is_some_and(|r| r.retail || r.use_.is_some() || usage_of(r).is_some_and(|u| f.js.consts.frontage_trade_usages.iter().any(|x| x == u)));
                if !ok {
                    continue;
                }
            }
            if let Some(wz) = &kind.work_zone {
                if c.last_placed.map(|l| f.kinds[l].id.as_str()) != Some(wz.as_str()) {
                    continue;
                }
            }
            let since = c.station - c.last_at.get(&k).copied().unwrap_or(-1e9);
            if since < kind.gap {
                continue;
            }
            if kind.shop_front == Some(false) && [0.0, -kind.radius, kind.radius].iter().any(|&a| (c.along)(a).is_some_and(|i| b.recs[i as usize].shop)) {
                continue;
            }
            let weight = if c.at_corner { kind.corner } else { kind.weight[class_band(c.class_rank)] };
            if !(weight > 0.0) {
                continue;
            }
            total += weight;
            legal.push((k, weight));
        }
        if legal.is_empty() {
            return None;
        }
        let mut cursor = draw * total;
        for &(k, wgt) in &legal {
            cursor -= wgt;
            if cursor <= 0.0 {
                return Some(k);
            }
        }
        legal.last().map(|l| l.0)
    }

    // -----------------------------------------------------------------------
    // surveyed placement
    // -----------------------------------------------------------------------

    /// `placeSurveyedTransit`.
    fn transit(&mut self, sc: &SideCtx, overlay: Option<&Overlay>, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let covered = f.transit_covers(e);
        let all_transit = |s: &mut HashSet<&'static str>| {
            s.insert("busStopFlag");
            s.insert("transitShelter");
        };
        let band = sc.band;
        let reach = band.outer + 4.0;
        let cell = f.js.consts.surveyed_cell_m;
        // `surveyedPointsForSegment`: the cells round each polyline vertex.
        let span = (reach / cell).ceil() as i32;
        let mut seen = HashSet::new();
        let mut nearby: Vec<u32> = vec![];
        for q in &e.points {
            let (cx, cz) = ((q[0] / cell).floor() as i32, (q[1] / cell).floor() as i32);
            for a in -span..=span {
                for b in -span..=span {
                    if seen.insert((cx + a, cz + b)) {
                        if let Some(v) = f.stops.cells.get(&(cx + a, cz + b)) {
                            nearby.extend_from_slice(v);
                        }
                    }
                }
            }
        }
        const STOP_END_SLACK_M: f64 = 3.0;
        let mut cands: Vec<(u32, f64, f64)> = vec![];
        for &id in &nearby {
            if self.claimed.contains(&(Layer::Stop, id)) {
                continue;
            }
            let p = f.stops.points[id as usize];
            let Some((station, distance, lateral)) = project(e, p.x, p.z) else { continue };
            if station < sc.s0 - STOP_END_SLACK_M || station > sc.s1 + STOP_END_SLACK_M {
                continue;
            }
            if lateral.abs() > reach || distance > reach.hypot(STOP_END_SLACK_M) {
                continue;
            }
            if lateral.abs() < e.half * 0.5
                && nearby.iter().any(|&q| q != id && {
                    let o = f.stops.points[q as usize];
                    (o.x - p.x).hypot(o.z - p.z) < 15.0
                })
            {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != sc.side {
                continue;
            }
            if !self.owns(Layer::Stop, id, [p.x, p.z]) {
                continue;
            }
            cands.push((id, station.clamp(sc.s0, sc.s1), lateral.abs()));
        }
        if cands.is_empty() {
            if covered {
                all_transit(suppress);
            }
            return;
        }
        cands.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
        const STOP_MERGE_M: f64 = 12.0;
        let mut stops: Vec<Vec<(u32, f64, f64)>> = vec![];
        for c in cands {
            match stops.last_mut() {
                Some(last) if c.1 - last[0].1 <= STOP_MERGE_M => last.push(c),
                _ => stops.push(vec![c]),
            }
        }
        let (mut placed_any, mut shelter_stated) = (false, false);
        let flag_k = f.k("busStopFlag");
        let shelter_k = f.k("transitShelter");
        let tree_k = f.k("tree");
        for stop in stops {
            let lead = stop.iter().find(|m| f.stops.points[m.0 as usize].attr != '-' as u32).copied().unwrap_or(stop[0]);
            let (pid, station, _) = lead;
            let point = f.stops.points[pid as usize];
            let mut flag = None;
            let mut flag_station = station;
            for &shift in &f.js.consts.stop_shifts {
                let at = station + shift;
                if at < sc.s0 || at > sc.s1 {
                    continue;
                }
                flag = self.place(sc, self.spec(at, flag_k));
                if flag.is_some() {
                    flag_station = at;
                    break;
                }
            }
            let Some(flag) = flag else { continue };
            for m in &stop {
                self.claimed.insert((Layer::Stop, m.0));
            }
            placed_any = true;
            if point.attr != '-' as u32 {
                shelter_stated = true;
            }
            self.mark_surveyed(Some(flag));
            // `surveyedStopSheltered`.
            let sheltered = if point.attr == 'S' as u32 || point.attr == 's' as u32 {
                true
            } else if point.attr != '-' as u32 {
                false
            } else {
                let h = hash32(&format!("{}:stop-shelter:{}:{}", f.seed, int_str(js_round(point.x * 10.0)), int_str(js_round(point.z * 10.0))));
                ((h % 1000) as f64) / 1000.0 < f.js.consts.unstated_shelter_share
            };
            if !sheltered {
                continue;
            }
            let shelter = &f.kinds[shelter_k];
            let flag_r = f.kinds[flag_k].radius;
            if band.walk < shelter.min_walk {
                self.shelter_fallback(sc, flag_station);
                continue;
            }
            let offset = shelter.radius + flag_r + 0.5;
            let trees = self.tree_candidates(sc, overlay);
            let tree_clear = shelter.radius + f.kinds[tree_k].radius;
            let clear_of_trees = |at: f64| trees.as_ref().is_none_or(|t| t.iter().all(|c| (c.1 - at).abs() >= tree_clear));
            let mut slots = vec![];
            for &shift in &f.js.consts.kerb_shifts {
                if shift < 0.0 {
                    continue;
                }
                slots.push(flag_station + offset + shift);
                slots.push(flag_station - offset - shift);
            }
            let mut ordered: Vec<f64> = slots.into_iter().filter(|&a| clear_of_trees(a)).collect();
            ordered.push(flag_station + offset);
            ordered.push(flag_station - offset);
            let mut tried = HashSet::new();
            let mut done = false;
            for at in ordered {
                let key = js_round(at * 100.0) as i64;
                if tried.contains(&key) || at < sc.s0 || at > sc.s1 {
                    continue;
                }
                tried.insert(key);
                let p = self.place(sc, self.spec(at, shelter_k));
                if p.is_some() {
                    self.mark_surveyed(p);
                    done = true;
                    break;
                }
            }
            if !done {
                self.shelter_fallback(sc, flag_station);
            }
        }
        if covered {
            all_transit(suppress);
        } else if placed_any {
            suppress.insert("busStopFlag");
            if shelter_stated {
                suppress.insert("transitShelter");
            }
        }
    }

    /// `placeSurveyedShelterFallback`.
    fn shelter_fallback(&mut self, sc: &SideCtx, flag_station: f64) {
        let f = self.f;
        let flag_r = f.kinds[f.k("busStopFlag")].radius;
        for id in f.js.consts.shelter_fallback_kinds.clone() {
            let Some(k) = f.kind_of(&id).map(|k| k as usize) else { continue };
            let kind = &f.kinds[k];
            if kind.catalogue_available == Some(false) || sc.band.walk < kind.min_walk {
                continue;
            }
            let offset = kind.radius + flag_r + 0.5;
            let mut placed = None;
            'outer: for &shift in &f.js.consts.kerb_shifts {
                if shift < 0.0 {
                    continue;
                }
                for at in [flag_station + offset + shift, flag_station - offset - shift] {
                    if at < sc.s0 || at > sc.s1 {
                        continue;
                    }
                    placed = self.place(sc, self.spec(at, k));
                    if placed.is_some() {
                        break 'outer;
                    }
                }
            }
            if placed.is_some() {
                self.mark_surveyed(placed);
                return;
            }
        }
    }

    /// `surveyedTreeCandidates`: (list id, station), or None where the list does not cover the ribbon.
    fn tree_candidates(&self, sc: &SideCtx, overlay: Option<&Overlay>) -> Option<Vec<(u32, f64)>> {
        let w = self.w;
        let e = &w.street.plan.entries[sc.ei];
        if !tree_covers(w, e) {
            return None;
        }
        let reach = sc.band.outer + 1.5;
        let absent = observed::windows(overlay, e, sc.side, "treesAbsent");
        let mut out = vec![];
        for id in self.along(&w.tree_cells, e, reach) {
            if self.claimed.contains(&(Layer::Tree, id)) {
                continue;
            }
            let t = self.f.tree(w, id);
            let Some((station, _, lateral)) = project(e, t.x, t.z) else { continue };
            if station < sc.s0 || station > sc.s1 {
                continue;
            }
            if lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != sc.side {
                continue;
            }
            if absent.iter().any(|win| station >= win.lo && station <= win.hi) {
                continue;
            }
            if !self.owns(Layer::Tree, id, [t.x, t.z]) {
                continue;
            }
            out.push((id, station));
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        Some(out)
    }

    /// `placeSurveyedTrees`.
    fn trees(&mut self, sc: &SideCtx, overlay: Option<&Overlay>, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let Some(cands) = self.tree_candidates(sc, overlay) else { return };
        let tree_k = f.k("tree");
        let palm_k = f.k("palm");
        let oak = crate::eztree::street_tree_species().ok().and_then(|s| s.iter().position(|x| x == "street-oak"));
        let variants = f.variant_count(tree_k);
        let seg = w.segment_of(e);
        for (id, station) in cands {
            let t = f.tree(w, id);
            let palm = t.form == "palm";
            // `surveyedAlleyTreeFit`.
            let tree_kind = &f.kinds[tree_k];
            let fit: Option<(KindSpec, usize)> = if !palm
                && seg.street_type.as_deref() == Some("alley")
                && t.form == "tall"
                && sc.band.walk < 2.3
                && oak.is_some()
                && variants > 0
            {
                let depth = 0.2;
                if sc.band.usable < depth * 2.0 + 0.06 + tree_kind.through.unwrap_or(0.0) {
                    None
                } else {
                    Some((KindSpec { radius: depth, depth, exclusive: Some(tree_kind.radius), ..tree_kind.clone() }, oak.unwrap() % variants))
                }
            } else {
                None
            };
            let mut placed = None;
            for &shift in &f.js.consts.kerb_shifts {
                let at = station + shift;
                if at < sc.s0 || at > sc.s1 {
                    continue;
                }
                let spec = Spec {
                    station: at,
                    kind: if palm { palm_k } else { tree_k },
                    fit: fit.as_ref().map(|x| &x.0),
                    forced: false,
                    node: None,
                    tree: Some(TreeSpec { variant: fit.as_ref().map(|x| x.1), form: Some(t.form.clone()), leaf: t.leaf.clone(), crown_m: t.crown_m, height_m: t.height_m }),
                };
                placed = self.place(sc, spec);
                if placed.is_some() {
                    break;
                }
                if shift == 0.0 && self.last_rejection != "overlap" {
                    break;
                }
            }
            if placed.is_none() {
                continue;
            }
            self.claimed.insert((Layer::Tree, id));
            self.mark_surveyed(placed);
        }
        // Overlay-added trees (`treesAdded`).
        for win in observed::windows(overlay, e, sc.side, "treesAdded") {
            let lo = win.lo;
            if lo < sc.s0 || lo > sc.s1 {
                continue;
            }
            for nudge in [0.0, 0.8, -0.8, 1.6, -1.6] {
                let spec = Spec {
                    tree: Some(TreeSpec {
                        variant: None,
                        form: Some(win.entry.form.clone().unwrap_or_else(|| "mature".into())),
                        leaf: None,
                        crown_m: Some(win.entry.crown_m.unwrap_or(0.0)),
                        height_m: Some(win.entry.height_m.unwrap_or(0.0)),
                    }),
                    ..self.spec(lo + nudge, tree_k)
                };
                if self.place(sc, spec).is_some() {
                    break;
                }
            }
        }
        suppress.insert("tree");
    }

    /// `placeSurveyedMeters`.
    fn meters(&mut self, sc: &SideCtx, overlay: Option<&Overlay>, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let seg = w.segment_of(e);
        if !survey_covers(seg.street_id.as_deref(), e, f.js.meters.bounds) {
            return;
        }
        let reach = sc.band.outer + 1.5;
        let mut cands: Vec<(u32, f64)> = vec![];
        for id in self.along(&f.meters.cells, e, reach) {
            if self.claimed.contains(&(Layer::Meter, id)) {
                continue;
            }
            let p = f.meters.points[id as usize];
            let Some((station, _, lateral)) = project(e, p.x, p.z) else { continue };
            if station < sc.s0 || station > sc.s1 || lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != sc.side {
                continue;
            }
            if !self.owns(Layer::Meter, id, [p.x, p.z]) {
                continue;
            }
            cands.push((id, station));
        }
        cands.sort_by(|a, b| a.1.total_cmp(&b.1));
        let kind_name = |p: &SPoint| f.js.meters.kinds.get(p.attr as usize).cloned().unwrap_or_default();
        let kiosks: Vec<SPoint> = cands.iter().map(|c| f.meters.points[c.0 as usize]).filter(|p| kind_name(p) == "payStation").collect();
        let absent = observed::absent_windows(overlay, e, sc.side);
        for (id, station) in cands {
            let p = f.meters.points[id as usize];
            let kn = kind_name(&p);
            if absent.as_ref().is_some_and(|a| observed::absent_at(a, &kn, station)) {
                continue;
            }
            if kn == "parkingMeter" && kiosks.iter().any(|k| (k.x - p.x).hypot(k.z - p.z) < 1.5) {
                continue;
            }
            let Some(k) = f.kind_of(&kn).map(|k| k as usize) else { continue };
            if sc.band.walk < f.kinds[k].min_walk {
                continue;
            }
            let mut placed = None;
            for nudge in [0.0, 0.7, -0.7, 1.4, -1.4] {
                let at = station + nudge;
                if at < sc.s0 || at > sc.s1 {
                    continue;
                }
                placed = self.place(sc, self.spec(at, k));
                if placed.is_some() {
                    break;
                }
            }
            if placed.is_none() {
                continue;
            }
            self.claimed.insert((Layer::Meter, id));
            self.mark_surveyed(placed);
        }
        suppress.insert("parkingMeter");
        suppress.insert("payStation");
    }

    /// `placeSurveyedHydrants`.
    fn hydrants(&mut self, sc: &SideCtx, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        if f.hydrants.points.is_empty() {
            return;
        }
        let reach = sc.band.outer + 2.0;
        let k = f.k("hydrant");
        let mut any = false;
        for id in self.along(&f.hydrants.cells, e, reach) {
            if self.claimed.contains(&(Layer::Hydrant, id)) {
                continue;
            }
            let p = f.hydrants.points[id as usize];
            let Some((station, _, lateral)) = project(e, p.x, p.z) else { continue };
            if station < sc.s0 || station > sc.s1 || lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != sc.side {
                continue;
            }
            if !self.owns(Layer::Hydrant, id, [p.x, p.z]) {
                continue;
            }
            let mut placed = None;
            for nudge in [0.0, 0.8, -0.8, 1.6, -1.6] {
                let at = station + nudge;
                if at < sc.s0 || at > sc.s1 {
                    continue;
                }
                placed = self.place(sc, self.spec(at, k));
                if placed.is_some() {
                    break;
                }
            }
            if placed.is_none() {
                continue;
            }
            self.claimed.insert((Layer::Hydrant, id));
            self.mark_surveyed(placed);
            any = true;
        }
        if any {
            suppress.insert("hydrant");
        }
    }

    /// `placeObservedKit`.
    fn observed_kit(&mut self, sc: &SideCtx, overlay: Option<&Overlay>, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let mut placed_hydrant = false;
        let runs: [(&str, &str, [f64; 5]); 4] = [
            ("hydrants", "hydrant", [0.0, 0.8, -0.8, 1.6, -1.6]),
            ("payStations", "payStation", [0.0, 0.6, -0.6, 1.2, -1.2]),
            ("", "mailbox", [0.0, 0.6, -0.6, 1.2, -1.2]),
            ("bollards", "bollard", [0.0, 0.4, -0.4, 0.8, -0.8]),
        ];
        for (key, kind, nudges) in runs {
            let k = f.k(kind);
            if kind == "mailbox" {
                // Overlay mailboxes at world points (any overlay, any street).
                for o in &f.js.overlays {
                    for b in &o.mailboxes {
                        let (Some(x), Some(z)) = (b.x, b.z) else { continue };
                        let Some((offset, station, side)) = observed::world_point_on_segment_side(e, x, z) else { continue };
                        if side != sc.side || station < sc.s0 || station > sc.s1 || offset > e.half + sc.band.outer + 3.0 {
                            continue;
                        }
                        for nd in nudges {
                            if self.place(sc, self.spec(station + nd, k)).is_some() {
                                break;
                            }
                        }
                    }
                }
                continue;
            }
            for win in observed::windows(overlay, e, sc.side, key) {
                if win.lo < sc.s0 || win.lo > sc.s1 {
                    continue;
                }
                for nd in nudges {
                    if self.place(sc, self.spec(win.lo + nd, k)).is_some() {
                        if kind == "hydrant" {
                            placed_hydrant = true;
                        }
                        break;
                    }
                }
            }
        }
        let k = f.k("bikeRack");
        for win in observed::windows(overlay, e, sc.side, "bikeRacks") {
            if win.lo < sc.s0 || win.lo > sc.s1 {
                continue;
            }
            for nd in [0.0, 0.5, -0.5, 1.0, -1.0] {
                if self.place(sc, self.spec(win.lo + nd, k)).is_some() {
                    break;
                }
            }
        }
        if placed_hydrant {
            suppress.insert("hydrant");
        }
    }

    /// `placeSurveyedDressing`.
    fn dressing(&mut self, sc: &SideCtx, overlay: Option<&Overlay>, suppress: &mut HashSet<&'static str>) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let reach = sc.band.outer + 2.0;
        let absent = observed::absent_windows(overlay, e, sc.side);
        for id in self.along(&f.dressing.cells, e, reach) {
            if self.claimed.contains(&(Layer::Dressing, id)) {
                continue;
            }
            let p = f.dressing.points[id as usize];
            let Some(kind_id) = f.js.consts.surveyed_dressing_kinds.get(DRESSING[p.kind as usize]) else { continue };
            let Some(k) = f.kind_of(kind_id).map(|k| k as usize) else { continue };
            let kind = &f.kinds[k];
            if kind.catalogue_available == Some(false) {
                continue;
            }
            let Some((station, _, lateral)) = project(e, p.x, p.z) else { continue };
            if station < sc.s0 || station > sc.s1 || lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != sc.side {
                continue;
            }
            if absent.as_ref().is_some_and(|a| observed::absent_at(a, kind_id, station)) {
                continue;
            }
            if !self.owns(Layer::Dressing, id, [p.x, p.z]) {
                continue;
            }
            let racks = if DRESSING[p.kind as usize] == "bicycleParking" && p.attr > 1 { p.attr as usize } else { 1 };
            let mut any = false;
            for r in 0..racks {
                let centre = station + (r as f64 - (racks as f64 - 1.0) / 2.0) * kind.radius * 2.0;
                let mut placed = None;
                for nudge in [0.0, 0.8, -0.8, 1.6, -1.6] {
                    let at = centre + nudge;
                    if at < sc.s0 || at > sc.s1 {
                        continue;
                    }
                    placed = self.place(sc, self.spec(at, k));
                    if placed.is_some() {
                        break;
                    }
                }
                if placed.is_some() {
                    any = true;
                    self.mark_surveyed(placed);
                }
            }
            if any {
                self.claimed.insert((Layer::Dressing, id));
            }
        }
        if !f.js.bike_racks.data.is_empty() {
            suppress.insert("bikeRack");
        }
    }

    /// `placeStreetLights` (the district lights; on, as in the game).
    fn street_lights(&mut self, sc: &SideCtx) {
        let w = self.w;
        let f = self.f;
        let e = &w.street.plan.entries[sc.ei];
        let seg = w.segment_of(e);
        let way = Furniture::way_id(seg.street_id.as_deref());
        if way.is_some_and(|id| f.dist_ways.contains(&id)) {
            return;
        }
        let hw = seg.highway.as_str();
        let cls = if hw.starts_with("primary") || hw.starts_with("trunk") {
            "primary"
        } else if hw.starts_with("secondary") {
            "secondary"
        } else if hw.starts_with("tertiary") {
            "tertiary"
        } else {
            "residential"
        };
        let Some(spacing) = f.js.street_lights.spacing.get(cls).and_then(|v| v["medianM"].as_f64()).filter(|v| *v != 0.0) else { return };
        let code = way.and_then(|id| f.light_ways.get(&id)).map(|c| c.to_string());
        let kind_id = code.and_then(|c| f.js.consts.street_light_kind.get(&c).cloned()).unwrap_or_else(|| "lampCobra".into());
        let k = f.k(&kind_id);
        let pitch = spacing * 2.0;
        let reach = (spacing / 3.0).floor() as i32;
        let mut station = sc.s0 + if sc.side > 0.0 { 0.0 } else { spacing };
        while station <= sc.s1 {
            for step in 0..=reach * 2 {
                let nudge = if step % 2 == 1 { ((step + 1) / 2) as f64 } else { -((step / 2) as f64) };
                let at = station + nudge;
                if at < sc.s0 || at > sc.s1 {
                    continue;
                }
                if self.place(sc, self.spec(at, k)).is_some() {
                    break;
                }
            }
            station += pitch;
        }
    }
}

struct ChooseCtx<'c> {
    band: &'c Band,
    class_rank: i32,
    at_corner: bool,
    signal_corner: bool,
    last_at: &'c HashMap<usize, f64>,
    station: f64,
    building_gap: f64,
    frontage: Option<u32>,
    suppress: &'c HashSet<&'static str>,
    parks: bool,
    absent: Option<&'c [observed::Window<'c>]>,
    last_placed: Option<usize>,
    along: &'c dyn Fn(f64) -> Option<u32>,
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

// ---------------------------------------------------------------------------
// the public surface
// ---------------------------------------------------------------------------

/// The hydrants one ribbon carries: (station, side, foot position). Used for
/// the 15 ft hydrant parking clearance.
pub fn hydrants_of_entry(w: &PropsWorld, ei: usize) -> Vec<(f64, i8, [f64; 3])> {
    let f = w.furniture();
    let Some(k) = f.kind_of("hydrant") else { return vec![] };
    f.entry(w, ei).items.iter().filter(|i| i.kind == k).map(|i| (i.station, i.side, [i.x, i.y, i.z])).collect()
}
