//! Stage W2b: street life (`--street-life 1|2`, off by default): the small
//! things that say people use a street. Litter, cigarette butts, dead leaves
//! and grit on the footways and in the gutters; graffiti tags, throw-ups,
//! buffed-over patches, wheat-pasted posters and (level 2) stickers on the
//! ground floors of the buildings at the building line and on freeway
//! columns; (level 2) utility locate marks on the walk and in the gutter.
//!
//! Everything is a street decal (`cc0_decals.rs`): placed per 128 m street
//! tile by deterministic rules on the street plan, streamed with the tile,
//! and drawn in the material path of the surface it lies on, so it is lit,
//! shadowed, wetted and fogged as that surface: the walk and the gutter in
//! the street material (cc0.wgsl `cc0_street_decals` calls
//! street_life.wgsl `sl_street_decals`), the kit buildings' surfaces after
//! their facade ageing, glass excepted (draw.wgsl calls `sl_kit_wall`), the
//! freeway columns in the structures' concrete (draw.wgsl kind 12 calls
//! `sl_structure`). Albedo,
//! roughness and a little relief only: no emission, nothing in the lighting.
//!
//! Two decal kinds, both appended after every other decal of a tile (so
//! with the level 0 a tile's records are exactly as before), packed after
//! them in the tile's records and listed in cells of their own (cc0.rs
//! `LIFE_GRID_VEC4`, allocated only with the level on), so they never take a
//! cell entry from the wear decals:
//!
//! * `DK_LIFE`: one per straight stretch of a run (at most `STRETCH_MAX_M`,
//!   heading within `STRETCH_TURN_DEG`) and side with a footway, from just
//!   off the gutter (`INNER_PAD_M` into the carriageway) to `WALL_REACH_M`
//!   past the building line. Its kind carries the stretch's densities in the
//!   bits above its low byte (`Params`); its seed the places of its anchors
//!   along it (`seed_of`: the two nearest street trees, a storm drain, a bin
//!   at a stop or the corner). The scatter itself is keyed on the world, and an item is
//!   drawn by the one decal whose own stretch holds its centre (the decals
//!   overlap by `PAD_M` at each end), so nothing is cut at a stretch's end
//!   or at a tile border.
//! * `DK_COLUMN`: one per freeway column (`--structures`), its seed the
//!   street datum beside it (the tags' heights are measured from it).
//!
//! Densities by land use (`LandUse`, the building at the building line, from
//! the building-metadata packs' `use`, else its OSM tags), foot traffic (the
//! surveyed transit stops and the shopfronts, `cc0_decals::Hotspots`), the
//! street's class, corners (`CORNER_M` from a run end with an intersection
//! approach), the street trees (the DataSF street tree list, eo-props) and
//! the freeway decks overhead (`--structures`). Each run side also draws its
//! own care (`CARE_*`): some blocks swept and buffed, some neglected. See
//! `params_for` for the numbers; the reference is the sheet's §5.10 (Mission
//! St sidewalk: gum, butts and paper in the crumbling kerb; Selby St under
//! I-280: graffiti on the column bases, debris; Mariposa St: heavy graffiti on
//! the piers).
//!
//! Stage W2c made it read at normal and mid viewing distance (W2b's tags were
//! thin lines and its litter vanished past ~30 m): bold graffiti (fat-cap and
//! fat-marker tags 2.5-7 cm a stroke, some two-tone, in a low and a high band;
//! throw-ups 0.5-1.25 m tall with drop shadows, drips and overspray; on the
//! hottest walls a field of old faded tags and multi-colour pieces with a 3D
//! block and a cloud; column bases covered to 2-3 m), litter that gathers in
//! clusters metres long with a dark wet halo (drifts against the kerb and the
//! building line, piles at the storm drains and by the corners' and stops'
//! bins), heaps at the wall's foot (bags, leaning cardboard, a mattress), and
//! denser tables at every level (level 2 one step denser again, `Params::at_level`).

use crate::cc0_decals::{Decal, Hotspots};
use eo_street::StreetWorld;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

/// Decal kinds (street_life.wgsl `SL_DK_*`; after `cc0_decals::DK_WALK_EDGE`).
pub const DK_LIFE: u32 = 14;
pub const DK_COLUMN: u32 = 15;
/// The kinds carry their parameters above the low byte (the shaders read `kind & 255`).
/// A kind is up to 2^24, an exact integer in the record's f32, read with `u32(z)`:
/// rounding it with + 0.5 would carry an odd kind into the next one (the f32's
/// spacing is 1 from 2^23).
pub const KIND_SHIFT: u32 = 8;

/// A stretch is at most this long (m): the densities vary along a block.
pub const STRETCH_MAX_M: f64 = 16.0;
/// ... and turns at most this much (degrees) over its length, so the decal's
/// straight rectangle stays on its kerb line (0.14 m off at its middle on a
/// 16 m chord at the most).
pub const STRETCH_TURN_DEG: f64 = 1.0;
/// Neighbouring stretches' decals overlap this much (m) at each end; an item
/// is drawn by the decal whose own stretch (without it) holds its centre.
/// More than the largest scatter item's half size (a cardboard sheet, 0.3 m).
pub const PAD_M: f64 = 0.35;
/// The decal reaches this far into the carriageway past the gutter pan (m):
/// litter blown off the gutter, a dropped cup in the parking lane.
pub const INNER_PAD_M: f64 = 0.6;
/// The decal reaches this far past the building line (m): the kit's walls
/// stand behind the footprint's edge in places (recessed shop fronts and
/// entries, measured up to 1.5 m on a retail strip's front).
pub const WALL_REACH_M: f64 = 2.0;
/// A wall this far behind the walk's back edge or nearer counts as the
/// building line (m); probed every `WALL_PROBE_STEP_M`. SoMa's warehouses
/// stand behind loading aprons and SF's houses behind front gardens and
/// stoops; past it no wall pieces (litter still drifts at the walk's edge).
pub const WALL_PROBE_M: f64 = 10.0;
pub const WALL_PROBE_STEP_M: f64 = 0.25;
/// Street trees within this of a stretch's walk (m) shed on it.
pub const TREE_REACH_M: f64 = 7.0;
/// A stretch within this of a run end with an intersection approach is a
/// corner (m): people wait to cross there.
pub const CORNER_M: f64 = 15.0;
/// A column's decal reaches this far past its radius (m).
pub const COLUMN_PAD_M: f64 = 0.3;
/// A deck soffit lower than this over the street datum (m) puts a stretch
/// under a freeway (the tallest SF viaducts: 15-20 m, reference sheet 5.10).
pub const UNDER_DECK_M: f64 = 30.0;
/// Each run side's care: this share swept and buffed often (densities down),
/// this share neglected (densities up).
pub const CARE_CLEAN_SHARE: f64 = 0.22;
pub const CARE_NEGLECT_SHARE: f64 = 0.14;
/// Stage W2c, the piles' anchors (street_life.wgsl `sl_clusters`): a storm
/// drain within this of a stretch's kerb line (m; cc0_decals snaps the inlets
/// from 2.5 m) gathers a pile in the gutter at its inlet; a transit stop within
/// this of the walk's middle line (m), else the corner (this far along from a
/// run end with an intersection approach, m: SF's public litter cans stand at
/// the corners and the stops), has a bin whose pile spills onto the walk by
/// the kerb, on this share of the stretches that have one (every one at
/// level 2). INFERRED (no bin placement is surveyed: eo-props places its
/// furniture on its own rules, not read here).
pub const DRAIN_REACH_M: f64 = 2.5;
pub const BIN_STOP_REACH_M: f64 = 4.0;
pub const BIN_FROM_CORNER_M: f64 = 3.5;
pub const BIN_PILE_SHARE: f64 = 0.7;

/// The level (0 off, 1 high, 2 hyper) street tiles place with (`cc0_decals::place`).
static LEVEL: AtomicU8 = AtomicU8::new(0);

/// eo-app, with `--street-life`, before the street streams.
pub fn set_level(level: u8) {
    LEVEL.store(level.min(2), Ordering::Relaxed);
}

pub fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

/// What a building is used for, as far as the street in front of it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LandUse {
    Unknown = 0,
    House = 1,
    Flats = 2,
    Office = 3,
    Retail = 4,
    Industrial = 5,
    Vacant = 6,
    /// Civic, worship, schools, hospitals, museums: kept clean, no graffiti.
    Civic = 7,
}

impl LandUse {
    pub const ALL: [LandUse; 8] = [LandUse::Unknown, LandUse::House, LandUse::Flats, LandUse::Office, LandUse::Retail, LandUse::Industrial, LandUse::Vacant, LandUse::Civic];

    pub fn from_u8(v: u8) -> LandUse {
        Self::ALL.get(v as usize).copied().unwrap_or(LandUse::Unknown)
    }

    /// The building-metadata packs' `use` (public/data/sf/building-metadata, `categories`).
    pub fn from_metadata(u: &str) -> Option<LandUse> {
        Some(match u {
            "house" => LandUse::House,
            "apartment" | "condo" => LandUse::Flats,
            "office" | "hotel" => LandUse::Office,
            // SF mixed use is flats over a shop: the street sees the shop.
            "mixed-use" | "retail" | "restaurant" | "cafe" | "bar" | "supermarket" | "convenience" | "gas-station" => LandUse::Retail,
            "industrial" | "warehouse" | "auto-shop" | "parking-garage" => LandUse::Industrial,
            "vacant" => LandUse::Vacant,
            "civic" | "church" | "school" | "hospital" | "museum" => LandUse::Civic,
            _ => return None,
        })
    }

    /// A building with no metadata record: its OSM `building` and `amenity` tags.
    pub fn from_osm(building: &str, amenity: &str) -> LandUse {
        match building {
            "industrial" | "warehouse" | "garage" | "garages" | "service" | "manufacture" => LandUse::Industrial,
            "retail" | "commercial" | "supermarket" | "kiosk" => LandUse::Retail,
            "house" | "detached" | "semidetached_house" | "terrace" => LandUse::House,
            "apartments" | "residential" | "dormitory" => LandUse::Flats,
            "office" | "hotel" => LandUse::Office,
            "church" | "cathedral" | "chapel" | "school" | "university" | "hospital" | "civic" | "public" | "government" | "museum" | "fire_station" | "train_station" | "transportation" => LandUse::Civic,
            _ => match amenity {
                "" => LandUse::Unknown,
                "place_of_worship" | "school" | "university" | "college" | "kindergarten" | "hospital" | "townhall" | "courthouse" | "library" | "police" | "fire_station" => LandUse::Civic,
                _ => LandUse::Retail,
            },
        }
    }

    /// Graffiti (tags, throw-ups, pieces from 5) on the ground floor, 0..7.
    /// Industrial and vacant fronts and roll-up doors draw most (the Selby and
    /// Mariposa St notes: walls and piers covered), shopfronts several tags in
    /// a few metres, homes' garage doors now and then, civic buildings none.
    /// (Stage W2c raised every class but civic by one or two: at W2b's values a
    /// SoMa block read nearly clean from across the street.)
    pub fn tags(self) -> i32 {
        match self {
            LandUse::Unknown => 2,
            LandUse::House => 1,
            LandUse::Flats => 2,
            LandUse::Office => 3,
            LandUse::Retail => 4,
            LandUse::Industrial => 6,
            LandUse::Vacant => 7,
            LandUse::Civic => 0,
        }
    }

    /// Wheat-pasted posters and flyers, 0..7: shutters and blank fronts of
    /// shops, warehouses and empty lots.
    pub fn posters(self) -> i32 {
        match self {
            LandUse::Retail => 2,
            LandUse::Industrial => 3,
            LandUse::Vacant => 5,
            LandUse::Office | LandUse::Unknown => 1,
            _ => 0,
        }
    }

    /// Litter on top of the street's own (class, foot traffic), -1..2.
    pub fn litter(self) -> i32 {
        match self {
            LandUse::House => -1,
            LandUse::Retail | LandUse::Industrial | LandUse::Vacant => 1,
            _ => 0,
        }
    }

    /// Grit and sand drifts, 0..3.
    pub fn grit(self) -> i32 {
        match self {
            LandUse::House | LandUse::Civic => 0,
            LandUse::Industrial | LandUse::Vacant => 2,
            _ => 1,
        }
    }
}

/// The densities of one stretch side, packed above a kind's low byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Params {
    /// Paper, cups, wrappers, butts: 0..7.
    pub litter: u32,
    /// Dead leaves: 0..7.
    pub leaves: u32,
    /// Grit and sand drifts against the kerb and the building line: 0..3.
    pub grit: u32,
    /// Tags and throw-ups on the wall: 0..7 (0 without a wall).
    pub tags: u32,
    /// Posters and flyers on the wall: 0..7 (0 without a wall).
    pub posters: u32,
    /// Under a freeway deck: cardboard, dust, pigeon droppings.
    pub deck: bool,
    /// A wall stands at the building line (within `WALL_PROBE_M` of the walk's back edge).
    pub wall: bool,
}

impl Params {
    pub fn pack(self) -> u32 {
        (self.litter & 7) | (self.leaves & 7) << 3 | (self.grit & 3) << 6 | (self.tags & 7) << 8 | (self.posters & 7) << 11 | (self.deck as u32) << 14 | (self.wall as u32) << 15
    }

    pub fn unpack(p: u32) -> Params {
        Params { litter: p & 7, leaves: p >> 3 & 7, grit: p >> 6 & 3, tags: p >> 8 & 7, posters: p >> 11 & 7, deck: p >> 14 & 1 == 1, wall: p >> 15 & 1 == 1 }
    }

    /// At `level` 2 (hyper) one more litter, tags and posters where the
    /// stretch already has some (the shaders draw a third more of each piece
    /// on top); level 1 as it is.
    pub fn at_level(self, level: u8) -> Params {
        if level < 2 {
            return self;
        }
        let up = |v: u32| if v > 0 { (v + 1).min(7) } else { 0 };
        Params { litter: (self.litter + 1).min(7), tags: up(self.tags), posters: up(self.posters), ..self }
    }
}

/// A place along a stretch (0 its start .. 1 its end) as a 4-bit code, 0 for
/// none (street_life.wgsl `sl_anchor` reads it back).
pub fn anchor_code(t: Option<f64>) -> u32 {
    t.map(|t| 1 + (t.clamp(0.0, 1.0) * 14.0).round() as u32).unwrap_or(0)
}

/// A life decal's seed: its anchors' 4-bit codes (the two nearest street
/// trees, the storm drain, the bin), an exact integer below 2^16 in the
/// record's f32. The shaders hash the decal's centre for their randomness.
pub fn seed_of(codes: [u32; 4]) -> f32 {
    (codes[0] & 15 | (codes[1] & 15) << 4 | (codes[2] & 15) << 8 | (codes[3] & 15) << 12) as f32
}

/// `seed_of` read back.
pub fn seed_codes(seed: f32) -> [u32; 4] {
    let s = seed as u32;
    [s & 15, s >> 4 & 15, s >> 8 & 15, s >> 12 & 15]
}

/// A decal's kind with its parameters (16 bits above the low byte: exact in the f32 record).
pub fn kind_with(kind: u32, params: u32) -> u32 {
    kind | (params & 0xffff) << KIND_SHIFT
}

/// The survey placement reads: each building's land use (in `City::buildings`
/// order, the street world's `BuildingIndex` order) and the freeway decks.
#[derive(Default)]
pub struct Survey {
    pub classes: Vec<u8>,
    pub structures: Option<Arc<eo_structures::StructureWorld>>,
}

static SURVEY: OnceLock<Survey> = OnceLock::new();

/// eo-app, once, with `--street-life` (on its own thread at load; a street
/// tile that places before it waits for it, at most `SURVEY_WAIT_S`).
pub fn set_survey(classes: Vec<u8>, structures: Option<Arc<eo_structures::StructureWorld>>) {
    let _ = SURVEY.set(Survey { classes, structures });
}

/// How long a street tile waits for the survey before it places without it.
pub const SURVEY_WAIT_S: f64 = 30.0;

fn survey() -> &'static Survey {
    static EMPTY: OnceLock<Survey> = OnceLock::new();
    let t = std::time::Instant::now();
    loop {
        if let Some(s) = SURVEY.get() {
            return s;
        }
        if t.elapsed().as_secs_f64() > SURVEY_WAIT_S {
            eprintln!("street life: no survey after {SURVEY_WAIT_S} s; placing with the street's own data");
            return EMPTY.get_or_init(Survey::default);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The street trees (DataSF street tree list as eo-props packs it), on a 32 m grid.
pub struct Trees {
    cell: f64,
    grid: HashMap<(i32, i32), Vec<[f64; 2]>>,
    pub count: usize,
}

impl Trees {
    pub fn from_points(points: &[[f64; 2]]) -> Trees {
        let cell = 32.0;
        let mut grid: HashMap<(i32, i32), Vec<[f64; 2]>> = HashMap::new();
        for p in points {
            grid.entry(((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32)).or_default().push(*p);
        }
        Trees { cell, grid, count: points.len() }
    }

    /// The surveyed trees (eo-props' `JsProps::trees`, at the ground scale's positions).
    pub fn get() -> &'static Trees {
        static T: OnceLock<Trees> = OnceLock::new();
        T.get_or_init(|| {
            let pts: Vec<[f64; 2]> = eo_props::JsProps::load()
                .map(|js| {
                    let t = &js.trees;
                    let st = t.stride.max(2);
                    (0..t.data.len() / st).map(|i| [t.data[i * st] as f64 / 10.0, t.data[i * st + 1] as f64 / 10.0]).collect()
                })
                .unwrap_or_default();
            Trees::from_points(&pts)
        })
    }

    /// Trees within `reach` of the segment a-b: (t along it 0..1, lateral distance).
    fn along(&self, a: [f64; 2], b: [f64; 2], reach: f64) -> Vec<(f64, f64)> {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = (dx * dx + dz * dz).max(1e-9);
        let (x0, x1) = (a[0].min(b[0]) - reach, a[0].max(b[0]) + reach);
        let (z0, z1) = (a[1].min(b[1]) - reach, a[1].max(b[1]) + reach);
        let mut out = vec![];
        for cz in (z0 / self.cell).floor() as i32..=(z1 / self.cell).floor() as i32 {
            for cx in (x0 / self.cell).floor() as i32..=(x1 / self.cell).floor() as i32 {
                for p in self.grid.get(&(cx, cz)).into_iter().flatten() {
                    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2;
                    if !(0.0..=1.0).contains(&t) {
                        continue;
                    }
                    let d = ((p[0] - a[0] - dx * t).powi(2) + (p[1] - a[1] - dz * t).powi(2)).sqrt();
                    if d <= reach {
                        out.push((t, d));
                    }
                }
            }
        }
        out.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.total_cmp(&y.0)));
        out
    }
}

/// What the placement reads beside the street world.
pub struct Context<'a> {
    pub level: u8,
    /// Per building (the street world's `BuildingIndex` order): `LandUse as u8`.
    pub classes: &'a [u8],
    pub structures: Option<&'a eo_structures::StructureWorld>,
    pub trees: &'a Trees,
    pub hot: &'a Hotspots,
}

impl Context<'static> {
    /// The level, the survey eo-app set, the tree list, the hotspots.
    pub fn global(level: u8) -> Context<'static> {
        let s = survey();
        Context { level, classes: &s.classes, structures: s.structures.as_deref(), trees: Trees::get(), hot: Hotspots::get() }
    }
}

/// What the placement found (bench JSON `street_life`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub buildings_classed: usize,
    pub trees: usize,
    pub structures: bool,
}

/// The survey's size, without waiting for it.
pub fn stats() -> Stats {
    match SURVEY.get() {
        Some(s) => Stats { buildings_classed: s.classes.iter().filter(|&&c| c != 0).count(), trees: if level() > 0 { Trees::get().count } else { 0 }, structures: s.structures.is_some() },
        None => Stats::default(),
    }
}

fn hash(a: u32, b: u32) -> f64 {
    let mut h = a.wrapping_mul(0x9e37_79b1) ^ b.wrapping_mul(0x85eb_ca77).rotate_left(13);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f64 / 16_777_216.0
}

/// The point at arc length `s` and lateral `u` of a run's stations, clamped to them.
fn at_s(g: &eo_street::mesh::EntryGeom, s: f64, u: f64) -> [f64; 2] {
    let st = &g.stations;
    let s = s.clamp(st[0].s, st[st.len() - 1].s);
    let i = st.partition_point(|f| f.s <= s).clamp(1, st.len() - 1);
    let (a, b) = (&st[i - 1], &st[i]);
    let t = if b.s > a.s { (s - a.s) / (b.s - a.s) } else { 0.0 };
    let (p, q) = (a.at(u), b.at(u));
    [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
}

/// The station frame's normal at arc length `s` (the span's start station).
fn normal_at(g: &eo_street::mesh::EntryGeom, s: f64) -> [f64; 2] {
    let st = &g.stations;
    let i = st.partition_point(|f| f.s <= s).clamp(1, st.len() - 1) - 1;
    [st[i].nx, st[i].nz]
}

/// A run's stretches, over the whole run (so every tile cuts them alike):
/// (start s, end s, first span, last span), the kept spans only, straight
/// within `STRETCH_TURN_DEG`, at most `STRETCH_MAX_M` (a longer straight run
/// is cut in equal pieces).
pub fn stretches(g: &eo_street::mesh::EntryGeom) -> Vec<(f64, f64, usize, usize)> {
    let st = &g.stations;
    let n = st.len();
    let keep = |i: usize| g.keep.get(i).copied().unwrap_or(true);
    let cos_max = STRETCH_TURN_DEG.to_radians().cos();
    let mut out = vec![];
    let mut i = 0;
    while i + 1 < n {
        if !keep(i) || st[i + 1].s - st[i].s < 1e-6 {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j + 1 < n && keep(j) && st[i].tx * st[j].tx + st[i].tz * st[j].tz >= cos_max && st[j + 1].s - st[i].s <= STRETCH_MAX_M {
            j += 1;
        }
        let (s0, s1) = (st[i].s, st[j].s);
        let k = ((s1 - s0) / STRETCH_MAX_M).ceil().max(1.0) as usize;
        for q in 0..k {
            let a = s0 + (s1 - s0) * q as f64 / k as f64;
            let b = s0 + (s1 - s0) * (q + 1) as f64 / k as f64;
            // the spans (station index ranges) the piece lies on
            let fa = st.partition_point(|f| f.s <= a).clamp(1, n - 1) - 1;
            let fb = (st.partition_point(|f| f.s < b).clamp(1, n - 1) - 1).max(fa);
            out.push((a, b, fa, fb));
        }
        i = j;
    }
    out
}

/// The densities of a stretch side (see the module doc). `draw`: the foot
/// traffic at its walk (0..1, `Hotspots::draw`); `busy`: the street class
/// (0.4 residential .. 1 primary); `trees`: the street trees beside it
/// (lateral distances); `care`: the run side's care draw (0..1).
#[allow(clippy::too_many_arguments)]
pub fn params_for(lu: LandUse, wall: bool, deck: bool, corner: bool, alley: bool, draw: f64, busy: f64, trees: &[(f64, f64)], care: f64) -> Params {
    let clean = care < CARE_CLEAN_SHARE;
    let neglect = care > 1.0 - CARE_NEGLECT_SHARE;
    // (stage W2c: a swept block keeps more than W2b's: litter -1, tags halved, was -2 and a third)
    let care_k = if clean { -1 } else if neglect { 2 } else { 0 };
    // Litter: the class (busier streets more), people (stops and shop doors), corners,
    // under a deck much more, the land use, the block's care.
    // (SF's own streets carry some everywhere: a residential block 3, a busy street 4;
    // stage W2c, from W2b's 2 and 3, which read swept past 10 m)
    let base = if busy >= 0.8 { 4 } else { 3 };
    let litter = base + (draw * 3.0).round() as i32 + corner as i32 + 3 * deck as i32 + lu.litter() + care_k;
    // Leaves: the trees beside it (one near: 4, closer and more: up to 7); a little blown
    // into homes' streets without trees of their own.
    let leaves = match trees.first() {
        Some(&(_, d)) => 4 + (d < 3.0) as i32 + (trees.len() >= 2) as i32 + (trees.len() >= 4) as i32,
        None => matches!(lu, LandUse::House | LandUse::Flats) as i32,
    };
    let grit = lu.grit() + 2 * deck as i32 + neglect as i32;
    // Wall pieces only on a wall at the building line; never on civic buildings.
    let (mut tags, mut posters) = (0, 0);
    if wall && lu != LandUse::Civic {
        tags = lu.tags() + 3 * deck as i32 + 2 * alley as i32;
        tags = if clean { tags / 2 } else if neglect { tags + 2 } else { tags };
        posters = lu.posters() + (draw > 0.5 && lu.posters() > 0) as i32 + neglect as i32;
    }
    let c = |v: i32, hi: i32| v.clamp(0, hi) as u32;
    // even a swept block keeps a few butts and torn bits: litter at least 1
    Params { litter: litter.clamp(1, 7) as u32, leaves: c(leaves, 7), grit: c(grit, 3), tags: c(tags, 7), posters: c(posters, 7), deck, wall }
}

/// Every street-life decal of the runs with spans in `key` and its
/// neighbours, and of the columns standing there, appended to `out`.
pub fn place(world: &StreetWorld, key: (i32, i32), cx: &Context, out: &mut Vec<Decal>) {
    if cx.level == 0 {
        return;
    }
    let mut spans: Vec<(u32, u32)> = vec![];
    for dz in -1..=1 {
        for dx in -1..=1 {
            if let Some(r) = world.tiles.get(&(key.0 + dx, key.1 + dz)) {
                spans.extend_from_slice(&r.spans);
            }
        }
    }
    spans.sort_unstable();
    spans.dedup();
    let o = &world.o;
    let mut k = 0;
    while k < spans.len() {
        let ei = spans[k].0;
        let mut j = k;
        while j + 1 < spans.len() && spans[j + 1].0 == ei {
            j += 1;
        }
        let here: Vec<u32> = spans[k..=j].iter().map(|s| s.1).collect();
        k = j + 1;
        let e = &world.plan.entries[ei as usize];
        let g = &world.geoms[ei as usize];
        if g.stations.len() < 2 {
            continue;
        }
        let run = eo_street::hash32(&e.id);
        let half = e.half;
        let busy = match e.highway.as_str() {
            "primary" | "trunk" | "primary_link" => 1.0,
            "secondary" | "secondary_link" => 0.8,
            "tertiary" | "tertiary_link" => 0.6,
            _ => 0.4,
        };
        let alley = matches!(e.highway.as_str(), "service" | "living_street" | "pedestrian") || half < 3.5;
        let (f0, f1) = (g.stations[0].s, g.stations[g.stations.len() - 1].s);
        for (pi, &(sa, sb, fa, fb)) in stretches(g).iter().enumerate() {
            if !here.iter().any(|&s| (s as usize) >= fa && (s as usize) <= fb) {
                continue;
            }
            let sm = (sa + sb) * 0.5;
            let corner = (e.approach_start.is_some() && sm - f0 < CORNER_M) || (e.approach_end.is_some() && f1 - sm < CORNER_M);
            for (wi, side) in [(0usize, 1.0f64), (1, -1.0)] {
                let walk = e.walks[wi];
                if !(walk >= o.min_sidewalk_width) {
                    continue;
                }
                let salt = (pi as u32) << 1 | wi as u32;
                let back = half + walk;
                // The building line: the first footprint within WALL_PROBE_M behind the walk,
                // probed at five places along the stretch (3.2 m apart at the most); its land
                // use: a civic building anywhere along it keeps the whole stretch's wall clean.
                let mut hits: Vec<(f64, u32)> = vec![];
                for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
                    let s = sa + (sb - sa) * t;
                    let mut d = 0.0;
                    while d <= WALL_PROBE_M + 1e-9 {
                        let p = at_s(g, s, side * (back + d));
                        if let Some(b) = world.buildings.at(p[0], p[1]) {
                            hits.push((d, b));
                            break;
                        }
                        d += WALL_PROBE_STEP_M;
                    }
                }
                let wall = hits.len() >= 3;
                let wall_d = if wall { hits.iter().map(|h| h.0).fold(f64::INFINITY, f64::min) - WALL_PROBE_STEP_M * 0.5 } else { 0.0 };
                let uses: Vec<LandUse> = hits.iter().map(|h| LandUse::from_u8(cx.classes.get(h.1 as usize).copied().unwrap_or(0))).collect();
                let lu = if uses.contains(&LandUse::Civic) { LandUse::Civic } else { uses.iter().copied().max_by_key(|l| (l.tags(), *l as u8)).unwrap_or(LandUse::Unknown) };
                let u_in = side * (half - o.gutter_width - INNER_PAD_M);
                let u_out = side * (back + wall_d.max(0.0) + WALL_REACH_M);
                let u_mid = (u_in + u_out) * 0.5;
                let (pa, pb) = (at_s(g, sa, u_mid), at_s(g, sb, u_mid));
                let (dx, dz) = (pb[0] - pa[0], pb[1] - pa[1]);
                let len = (dx * dx + dz * dz).sqrt();
                if len < 0.5 {
                    continue;
                }
                let mut a = [dx / len, dz / len];
                // +across (the decal's (-a.z, a.x)) points away from the road, toward the building line.
                let n = normal_at(g, sm);
                let flipped = (-a[1]) * n[0] * side + a[0] * n[1] * side < 0.0;
                if flipped {
                    a = [-a[0], -a[1]];
                }
                let walk_mid = at_s(g, sm, side * (half + walk * 0.5));
                let draw = cx.hot.draw(walk_mid);
                let deck = cx.structures.is_some_and(|sw| under_deck(world, sw, walk_mid));
                let (wa, wb) = (at_s(g, sa, side * (half + walk * 0.5)), at_s(g, sb, side * (half + walk * 0.5)));
                let trees = cx.trees.along(wa, wb, TREE_REACH_M);
                let care = hash(run ^ 0x51f3_a7c1, wi as u32);
                let p = params_for(lu, wall, deck, corner, alley, draw, busy, &trees, care).at_level(cx.level);
                // The seed: the anchors' places along the stretch as the decal runs (its
                // axis may run from sb to sa): the two nearest trees, the nearest storm drain
                // at the kerb, the bin at a stop or the corner (some stretches' only).
                let own = |t: f64| if flipped { 1.0 - t } else { t };
                let drain = cx.hot.drains_along(at_s(g, sa, side * half), at_s(g, sb, side * half), DRAIN_REACH_M).first().map(|d| own(d.0));
                let bin = if cx.level >= 2 || hash(run ^ 0x6a09_e667, salt) < BIN_PILE_SHARE {
                    let stop = cx.hot.stops_along(wa, wb, BIN_STOP_REACH_M).first().map(|s| s.0);
                    let at_end = |s: f64| (s > sa && s < sb).then(|| (s - sa) / (sb - sa));
                    let start = if e.approach_start.is_some() { at_end(f0 + BIN_FROM_CORNER_M) } else { None };
                    let near_corner = start.or(if e.approach_end.is_some() { at_end(f1 - BIN_FROM_CORNER_M) } else { None });
                    stop.or(near_corner).map(own)
                } else {
                    None
                };
                let seed = seed_of([anchor_code(trees.first().map(|t| own(t.0))), anchor_code(trees.get(1).map(|t| own(t.0))), anchor_code(drain), anchor_code(bin)]);
                out.push(Decal {
                    c: [((pa[0] + pb[0]) * 0.5) as f32, ((pa[1] + pb[1]) * 0.5) as f32],
                    a: [a[0] as f32, a[1] as f32],
                    half: [(len * 0.5 + PAD_M) as f32, ((u_out - u_in).abs() * 0.5) as f32],
                    kind: kind_with(DK_LIFE, p.pack()),
                    seed,
                });
            }
        }
    }
    // Freeway columns (`--structures`): tags and throw-ups round the base of most.
    if let Some(sw) = cx.structures {
        for dz in -1..=1 {
            for dx in -1..=1 {
                for f in sw.column_footprints((key.0 + dx, key.1 + dz)) {
                    let h = hash((f.x * 10.0).round() as i64 as u32, (f.z * 10.0).round() as i64 as u32 ^ 0x6c1d);
                    // Most columns tagged, their bases covered to 2-3 m (the Selby and Mariposa
                    // St notes), a few buffed clean.
                    let tags = if h < 0.12 { 1 } else { 5 + (h * 3.0) as u32 };
                    let r = f.radius + COLUMN_PAD_M;
                    let datum = world.ground(f.x, f.z) + o.road_lift;
                    out.push(Decal { c: [f.x as f32, f.z as f32], a: [1.0, 0.0], half: [r as f32, r as f32], kind: kind_with(DK_COLUMN, Params { tags: tags.min(7), deck: true, wall: true, ..Default::default() }.pack()), seed: datum as f32 });
                }
            }
        }
    }
}

/// Whether a deck or a cap stands over the street at (x, z), lower than `UNDER_DECK_M`.
fn under_deck(world: &StreetWorld, sw: &eo_structures::StructureWorld, p: [f64; 2]) -> bool {
    let y = world.ground(p[0], p[1]) + world.o.road_lift;
    sw.headroom(p[0], p[1], 0.5, y + 0.5).is_some_and(|h| h > y + 2.0 && h - y < UNDER_DECK_M)
}

/// WGSL constants (street_life.wgsl): the switch's bits, the kinds, the decals' geometry.
pub fn wgsl() -> String {
    format!(
        "\n// generated by street_life.rs\nconst SL_RUN_SHIFT: u32 = {}u;\nconst SL_DK_LIFE: u32 = {DK_LIFE}u;\nconst SL_DK_COLUMN: u32 = {DK_COLUMN}u;\nconst SL_KIND_SHIFT: u32 = {KIND_SHIFT}u;\nconst SL_PAD_M: f32 = {PAD_M:.4};\nconst SL_WALL_REACH_M: f32 = {WALL_REACH_M:.4};\nconst SL_COLUMN_PAD_M: f32 = {COLUMN_PAD_M:.4};\nconst SL_LIFE_BASE: u32 = {}u;\n",
        crate::facade::RUN_STREET_LIFE_SHIFT,
        crate::cc0::GRID_VEC4
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_pack_round_trip() {
        for i in 0..4096u32 {
            let p = Params { litter: i & 7, leaves: i >> 3 & 7, grit: i >> 6 & 3, tags: i >> 8 & 7, posters: (i * 7) & 7, deck: i & 1 == 1, wall: i & 2 == 2 };
            assert_eq!(Params::unpack(p.pack()), p);
            // the kind keeps its low byte and stays an exact integer in an f32 record
            for kind in [DK_LIFE, DK_COLUMN] {
                let k = kind_with(kind, p.pack());
                assert_eq!(k & 255, kind);
                assert_eq!((k as f32) as u32, k);
                assert_eq!(Params::unpack(k >> KIND_SHIFT), p);
            }
        }
    }

    #[test]
    fn densities_follow_land_use() {
        let t: &[(f64, f64)] = &[];
        let mid = 0.5;
        let p = |lu, deck| params_for(lu, true, deck, false, false, 0.0, 0.4, t, mid);
        assert_eq!(p(LandUse::Civic, false).tags, 0, "no graffiti on civic buildings");
        assert_eq!(p(LandUse::Civic, true).tags, 0, "not even under a deck");
        assert!(p(LandUse::Industrial, false).tags > p(LandUse::Retail, false).tags);
        assert!(p(LandUse::Retail, false).tags > p(LandUse::Flats, false).tags);
        assert!(p(LandUse::Flats, false).tags >= p(LandUse::House, false).tags);
        assert!(p(LandUse::Industrial, true).tags > p(LandUse::Industrial, false).tags, "more under a deck");
        assert!(p(LandUse::Industrial, true).litter > p(LandUse::Industrial, false).litter);
        // no wall: no wall pieces
        let nw = params_for(LandUse::Industrial, false, true, false, false, 1.0, 1.0, t, mid);
        assert_eq!((nw.tags, nw.posters, nw.wall), (0, 0, false));
        // foot traffic and corners draw litter; trees draw leaves
        let quiet = params_for(LandUse::Flats, true, false, false, false, 0.0, 0.4, t, mid);
        let busy = params_for(LandUse::Flats, true, false, true, false, 1.0, 1.0, t, mid);
        assert!(busy.litter >= quiet.litter + 3, "{quiet:?} {busy:?}");
        let leafy = params_for(LandUse::Flats, true, false, false, false, 0.0, 0.4, &[(0.5, 1.5), (0.1, 4.0)], mid);
        assert!(leafy.leaves >= 5 && quiet.leaves <= 1);
        // care: a swept block less, a neglected one more
        let clean = params_for(LandUse::Industrial, true, false, false, false, 0.5, 0.8, t, 0.0);
        let neglect = params_for(LandUse::Industrial, true, false, false, false, 0.5, 0.8, t, 0.999);
        assert!(clean.tags < neglect.tags && clean.litter < neglect.litter);
        for lu in LandUse::ALL {
            assert_eq!(LandUse::from_u8(lu as u8), lu);
        }
    }

    #[test]
    fn seeds_carry_their_anchors_exactly() {
        for i in 0..65536u32 {
            let codes = [i & 15, i >> 4 & 15, i >> 8 & 15, i >> 12 & 15];
            let s = seed_of(codes);
            // an exact integer in the record's f32 (the shader reads `u32(floor(seed))`)
            assert_eq!(s as u32, i);
            assert_eq!(s.floor(), s);
            assert_eq!(seed_codes(s), codes);
        }
        assert_eq!(anchor_code(None), 0);
        assert_eq!(anchor_code(Some(0.0)), 1);
        assert_eq!(anchor_code(Some(1.0)), 15);
        assert_eq!(anchor_code(Some(2.0)), 15, "clamped");
        // back to a place along the stretch (street_life.wgsl `sl_anchor`): within half a step
        for k in 0..=100 {
            let t = k as f64 / 100.0;
            let back = (anchor_code(Some(t)) - 1) as f64 / 14.0;
            assert!((back - t).abs() <= 0.5 / 14.0 + 1e-9, "{t} -> {back}");
        }
    }

    #[test]
    fn level_two_is_denser_and_level_one_as_placed() {
        let t: &[(f64, f64)] = &[];
        for lu in LandUse::ALL {
            let p = params_for(lu, true, false, false, false, 0.3, 0.6, t, 0.5);
            assert_eq!(p.at_level(1), p);
            let q = p.at_level(2);
            assert!(q.litter >= p.litter && q.tags >= p.tags && q.posters >= p.posters, "{lu:?}");
            assert!(q.litter > p.litter || p.litter == 7);
            // nothing appears where the stretch has none: no graffiti on civic walls even at level 2
            assert_eq!(q.tags == 0, p.tags == 0);
            assert_eq!(q.posters == 0, p.posters == 0);
        }
        // W2c's tables: a shop front carries several tags, an industrial or vacant front
        // pieces (5 and up), homes now and then, civic buildings none
        let at = |lu| params_for(lu, true, false, false, false, 0.0, 0.4, t, 0.5);
        assert!(at(LandUse::Retail).tags >= 4 && at(LandUse::Industrial).tags >= 5 && at(LandUse::Vacant).tags >= 5);
        assert!(at(LandUse::House).tags >= 1 && at(LandUse::Civic).tags == 0);
        assert!(at(LandUse::Flats).litter >= 3, "a residential block carries some litter");
    }

    #[test]
    fn metadata_and_osm_uses_map() {
        assert_eq!(LandUse::from_metadata("warehouse"), Some(LandUse::Industrial));
        assert_eq!(LandUse::from_metadata("mixed-use"), Some(LandUse::Retail));
        assert_eq!(LandUse::from_metadata("church"), Some(LandUse::Civic));
        assert_eq!(LandUse::from_metadata("something-else"), None);
        assert_eq!(LandUse::from_osm("yes", ""), LandUse::Unknown);
        assert_eq!(LandUse::from_osm("yes", "restaurant"), LandUse::Retail);
        assert_eq!(LandUse::from_osm("yes", "school"), LandUse::Civic);
        assert_eq!(LandUse::from_osm("industrial", ""), LandUse::Industrial);
    }

    #[test]
    fn the_constants_parse() {
        let src = wgsl();
        wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    }
}
