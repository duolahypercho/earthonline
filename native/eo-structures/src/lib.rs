//! Grade-separated roads: freeways, ramps, road bridges and tunnels.
//!
//! This crate owns their vertical profiles, their deck, support and portal
//! meshes, and their collision triangles. It reads the street plan
//! (`eo_street::StreetWorld`), the city's roads (`eo_data::City`, joined by
//! `street_id` for `layer`, `bridge` and `tunnel`), the street standards
//! (`eo_data::standards::CaStandards`) and the landmark bridges
//! (`eo_landmarks::LandmarksJs`); it never changes the street surface. No GPU.
//!
//! Everything is behind eo-app's `--structures` (and `--tunnels` for the
//! bores and portals); without it nothing is built and every consumer keeps
//! its `None`.
//!
//! What `build` does (stage A1, profiles):
//!
//! * the street plan's separated vehicle segments (`Segment::separated`,
//!   vehicle classes only) are joined to their `eo_data::Road` for `layer`,
//!   `bridge`, `tunnel`; the ways the two landmark bridges carry become
//!   landmark chains whose profile is the landmark's own deck;
//! * the segments are grouped into chains through shared end points (the
//!   plan's `node_snap`), split where more than two meet, where the level or
//!   the class changes, and where a paved street or a landmark deck end is
//!   met;
//! * bents are fixed along every deck chain (target span `MAX_SPAN_M`), each
//!   where a column has room, preferring off paving, then a street's
//!   median, then its parking lane or kerb-side furniture zone, then a
//!   straddle bent over it (`bents.rs`; never a travel or bike lane, a
//!   junction, a crosswalk or the throughway, never under another deck),
//!   the span lengthening where none has room; so the superstructure depth
//!   (`superstructure-depth-min-span-ratio`) is known per span; a chain those
//!   rules leave a span longer than `LONGEST_SPAN_M` is placed again by the
//!   relaxed rule (stage A5: a junction kept clear by the street as drawn
//!   rather than by the disc of its widest approach, a column in a divided
//!   freeway's barrier median, and shared bents with a parallel twin deck of
//!   the same route: one line of columns across both decks under one cap),
//!   and held if still left with one;
//! * one height profile per chain is solved, lowest level first: the lowest
//!   profile within the class's grade limit that clears every street (as
//!   drawn: datum, crown, junction pads) and lower deck it crosses by the
//!   table's clearance plus the depth, never goes into the ground (nor its
//!   superstructure into open water), and meets the street as drawn at a
//!   paved end or the landmark's deck; tunnels the mirror image under the
//!   ground. A paved or lower piece of the deck's own route running along
//!   under it, joined to it, is not crossed (`route.rs`). A chain that
//!   cannot is held, with its reason, and not built;
//! * same-level decks whose roadways overlap (a ramp's gore with its
//!   mainline, twin carriageways side by side) are solved again tied
//!   (stage A5, `solve::merge_ties`): the minor deck is pinned to the major
//!   where they overlap and the major never under what the minor needs
//!   there, so the two meet as one surface (`Chain::merged_with`); a pair
//!   the free solve put a clearance and a depth apart is a genuine
//!   over/under and left alone.
//!
//! The meshes (stage A2: `render_tile`, `surface_tile`, `column_footprints`)
//! sweep each built chain's cross-section along its stations (`sweep.rs`,
//! `section.rs`, `mesh.rs`): deck, paint, joints, Type 60 barriers, fascia,
//! box girder, bents, approach fill and abutments, end faces; three levels
//! of detail per 128 m tile (`tiles.rs`), built on first asking and kept.
//! A shared bent draws one cap under both decks and its columns once; at a
//! merge the minor deck's top stops at the major's fascia, warped to meet
//! its surface, with no barrier between them and no paint over the major's
//! (stage A5's gore). Landmark decks get collision only.
//!
//! Stage A6 (`joins.rs`, `build_joined`): the street is built a second time
//! with what the structures ask of it (`eo_street::joins`): a structure end
//! where streets pass through or meet is a leg of a junction there, the deck
//! starting at its mouth; a stub ending at a structure is capped; a short
//! bridge held as end too low is asked an approach embankment on its stub
//! (a ramp up to the end height it needs, within its own grade), and built
//! on it; a column may stand across two runs' kerbless parking lanes where
//! they overlap (13th Street under the Central Freeway, `bents.rs`).
//!
//! Stage A7 (`tunnel.rs`, `--tunnels`): tunnels draw and collide: the bore
//! (roadway, walkways, walls, vault in cream tile, a crown luminaire, a
//! centre wall where twin bores run side by side), the approach cuts and
//! their retaining walls, portal headwalls; lights along the crown
//! (`tunnel_lights`); the ground the cuts go through (`terrain_cuts`), for
//! the terrain and the collision heightfield to leave out; a stub at a
//! portal is asked to come down its cut (`StructureEnd::cut`).
//!
//! The clearance envelope (stage A6, `clear.rs`: `obstructions`, `blocked`,
//! `headroom`) answers what space the drawn decks, girders, barriers, fill
//! walls, caps and columns take over a disc, so the props keep out of it;
//! `barrier_at` gives the barrier tops the deck luminaires stand on.
//!
//! Tiles are the street's 128 m tiles (`eo_street::tile_key`,
//! `eo_street::STREET_TILE_M`). Heights are in the game frame (the terrain
//! already lifted by the height scale, the plan already scaled by the ground
//! scale), as the street surface's are; clearances and depths are the
//! table's values, unscaled.

mod bents;
mod east_span;
pub use east_span::{pavement_enabled as east_span_pavement, pavement_joints as east_span_paving_joints};
/// Deck colour alpha reserved for the registered eastern carriageways.
pub const EAST_SPAN_PAVING_ALPHA: u8 = 254;
pub mod skyway_lights;
mod census;
pub mod cut_ground;
mod chains;
mod clear;
mod index;
mod joins;
mod landmark;
mod mesh;
mod merge_surface;
mod profile;
mod route;
mod section;
mod solve;
mod sweep;
mod tiles;
mod tunnel;
mod tunnel_clearance;
mod island_section;
pub use island_section::IslandTunnelSection;
mod walls;
#[cfg(test)]
mod joins_tests;
#[cfg(test)]
mod mesh_tests;
#[cfg(test)]
mod synthetic;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use eo_data::City;
use eo_data::standards::CaStandards;
use eo_landmarks::LandmarksJs;
use eo_street::StreetWorld;
use eo_street::plan::{Frame, frame_at};

pub use census::{Census, Extreme, Km};
pub use joins::{JoinStats, Joined, LEG_LINE_M, RISE_MARGIN_M, build_joined, rebuild_streets};
pub use clear::{BarrierPoint, ENVELOPE_PAD_M, ENVELOPE_PAD_Y_M, Obstruction, Part};

/// Target span between bents (plan metres). INFERRED, structure-only: the
/// median of the mean spans the FHWA National Bridge Inventory records for
/// San Francisco freeway viaducts (tmp reference sheet section 1.1: 34 0026
/// I-280 Alemany 28.3 m, 34 0077 US-101 interchange 29.8 m, 34 0126L I-80 WB
/// West Approach 33.4 m, 34 0126R I-80 EB 37.7 m, 34 0177 Central Viaduct
/// 43.4 m; mean span = total length / main spans).
pub const MAX_SPAN_M: f64 = 33.4;

/// The longest span a chain's bents may leave when paving or a lower
/// roadway blocks a bent (the span lengthens to the next clear station);
/// a chain with a longer span is held (`Hold::NoBentRoom`). INFERRED,
/// structure-only: the longest freeway spans the reference sheet records in
/// the city, the Presidio Viaduct's three 275 ft (83.8 m) main spans
/// (section 1.2, Roads & Bridges 2011, ENR 2012).
pub const LONGEST_SPAN_M: f64 = 83.8;

/// Profile stations are no further apart than this along a chain (a solve
/// resolution, not a street dimension).
pub const STATION_MAX_M: f64 = 4.0;

/// Length of the short vertical curve (a parabola) put over a kink where the
/// grade turns by twice the limit; smaller turns get proportionally shorter
/// curves. INFERRED: the table has no vertical-curve length (Highway Design
/// Manual 204.4 is not in it), so this is a smoothing length only: the
/// curves never lower a deck under its bound nor steepen it past its limit.
pub const VERTICAL_CURVE_M: f64 = 30.0;

/// Least cover over a tunnel's roof. INFERRED, structure-only: the shallow
/// cut-and-cover tunnels under landscaped lids (Presidio Parkway, reference
/// sheet section 1.7) carry a soil layer of this order.
pub const MIN_COVER_M: f64 = 2.0;

/// A tunnel bore's height from the road to the top of its roof. INFERRED,
/// structure-only: the Broadway Tunnel's clear height, nearly 20 ft (6.1 m)
/// through most of the bore (reference sheet section 1.7, SF Chronicle 2006).
pub const BORE_CLEAR_M: f64 = 6.1;

/// The bore's lining tint and its luminaire strip's (sRGB; `tunnel.rs`), so
/// a renderer can tell them from the decks' concrete and joints (stage A8).
pub const TUNNEL_TILE_TINT: [u8; 3] = tunnel::TILE_TINT;
pub const TUNNEL_LUMINAIRE_RGB: [u8; 3] = tunnel::LUMINAIRE_RGB;

/// How far a bent's column footprint reaches past the deck edge, and so how
/// far it is kept from paving and from a lower deck's roadway. INFERRED,
/// structure-only: half the 2.1 m CIDH pile shaft under the Central
/// Viaduct's columns (reference sheet section 1.2, Kim 2005).
pub const BENT_FOOTPRINT_MARGIN_M: f64 = 1.05;

/// A point is on a deck when it is within the deck's half width plus this
/// (`deck_at`; a query tolerance).
pub const DECK_QUERY_MARGIN_M: f64 = 0.5;

/// How far past the deck edge a straddle bent's outrigger cap may reach to
/// put its columns outside a street running under the deck. INFERRED,
/// structure-only: the reference sheet records no outrigger length; this
/// is about a two-lane street's half width (the cap then spans the street
/// between columns on its far kerbs, as the straddle bents of the city's
/// older double-deck viaducts did).
pub const OUTRIGGER_REACH_M: f64 = 8.0;

/// Column diameter over superstructure depth: the middle of Caltrans SDC
/// v2.0 section 7.6.2's 0.70 <= Dc/Ds <= 1.00 for integral bent caps
/// (reference sheet section 1.3, Y).
pub const COLUMN_DC_OVER_DS: f64 = 0.85;

/// Least column diameter on a freeway mainline (motorway, trunk): the round
/// columns seen under I-280 at Selby Street, about 1.8-2.2 m across
/// (reference sheet section 5.10, first-hand photo notes). INFERRED.
pub const COLUMN_MIN_MAINLINE_M: f64 = 1.8;

/// Least column diameter on ramps and other roads. INFERRED,
/// structure-only: 0.70 x the depth (`cipBoxContinuous`) of a 43 m span,
/// the longest mean span the NBI records in the city (sheet 1.1, 1.3).
pub const COLUMN_MIN_OTHER_M: f64 = 1.2;

/// A deck up to this wide (barriers included) stands on one column per bent
/// under a T cap; up to `TWO_COLUMN_DECK_MAX_M` on two; wider on three.
/// INFERRED, structure-only: the narrow ramps on single-column T bents and
/// the wide decks on two or three columns (sheet 5.10 Selby St; the 27 m
/// Central Viaduct on three columns, sheet 1.2).
pub const ONE_COLUMN_DECK_MAX_M: f64 = 13.5;
pub const TWO_COLUMN_DECK_MAX_M: f64 = 24.5;

/// Which rule placed a bent, in the order they are tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BentRule {
    /// Every column off paving (A1's rule).
    OffPaving,
    /// A column in the median of a street under the deck.
    Median,
    /// A column in a parking lane or the kerb-side furniture zone of a
    /// street under the deck (never a travel lane, a bike lane, a junction,
    /// a crosswalk or the pedestrian throughway).
    Kerbside,
    /// Columns on both sides of the street running under the deck, outside
    /// its travel lanes, under a cap that may reach past the deck edge.
    Straddle,
}

impl BentRule {
    pub const ALL: [BentRule; 4] = [BentRule::OffPaving, BentRule::Median, BentRule::Kerbside, BentRule::Straddle];

    pub fn name(self) -> &'static str {
        match self {
            BentRule::OffPaving => "off paving",
            BentRule::Median => "median",
            BentRule::Kerbside => "parking lane or furniture zone",
            BentRule::Straddle => "straddle",
        }
    }
}

/// Where one column stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColumnSite {
    OffPaving,
    Median,
    /// In a street's parking lane (A6 keeps parked cars off it).
    ParkingLane,
    /// In the kerb-side furniture zone of a street's footway (A6 keeps
    /// trees, lamps and furniture off it).
    FurnitureZone,
}

impl ColumnSite {
    pub fn name(self) -> &'static str {
        match self {
            ColumnSite::OffPaving => "off paving",
            ColumnSite::Median => "median",
            ColumnSite::ParkingLane => "parking lane",
            ColumnSite::FurnitureZone => "furniture zone",
        }
    }

    /// On a street's paving (a parking lane, a furniture zone, a median).
    pub fn on_street(self) -> bool {
        self != ColumnSite::OffPaving
    }
}

/// The paved street run a column stands in.
#[derive(Clone, Debug, PartialEq)]
pub struct StreetRef {
    /// Index into `StreetWorld::plan.entries`.
    pub entry: u32,
    /// The run's id (`Entry::id`) and the street's name.
    pub id: String,
    pub name: String,
    /// Its side of the run: 0 = the run's +n (left, `walks[0]`, `parking[0]`), 1 = -n.
    pub side: u8,
}

/// One round column of a bent.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Lateral offset along the chain's frame normal at the bent station.
    pub u: f64,
    /// Plan position (world metres).
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub site: ColumnSite,
    /// The street it stands in (every site but `OffPaving`).
    pub street: Option<StreetRef>,
    /// The ground under it (elevation grid; the seabed over the bay).
    pub ground: f64,
}

/// How a bent's columns meet the superstructure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapKind {
    /// A deep rectangular cap beam under the soffit, cantilevering both
    /// sides of a single column (narrow decks, nothing under it to clear).
    TBeam,
    /// The cap is inside the box girder (flush with the soffit); each column
    /// meets the soffit through a square capital block.
    Integral,
    /// A flush cap reaching past the box to columns outside it (straddle bents).
    Outrigger,
}

/// One bent: its station, the rule that placed it, its columns and cap.
#[derive(Clone, Debug, PartialEq)]
pub struct Bent {
    pub s: f64,
    pub rule: BentRule,
    pub cap: CapKind,
    /// Columns, by lateral offset. Empty where the superstructure sits on
    /// its approach fill there (no room under it for a column).
    pub columns: Vec<Column>,
    /// Cap width along the chain: column diameter + 2 ft (Caltrans SDC
    /// Eq. 7.10, sheet 1.3).
    pub cap_width: f64,
    /// Depth of a `TBeam` cap below the soffit (0 for the others).
    pub cap_depth: f64,
    /// An expansion joint crosses the deck over this bent.
    pub joint: bool,
    /// A deck drain's downspout runs down its column nearest the +n edge.
    pub downspout: bool,
    /// A bent shared with a parallel twin deck of the same route (stage A5):
    /// one line of columns across both decks under one cap. None for a
    /// bent of this deck alone.
    pub twin: Option<BentTwin>,
}

/// The other half of a shared bent (`Bent::twin`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BentTwin {
    /// The other deck and its bent on the same line.
    pub chain: u32,
    pub bent: u32,
    /// The other deck's centreline along the line, as a lateral offset in
    /// the frame of the bent that owns the columns.
    pub u: f64,
    /// This deck rests on the other's cap: the columns and the cap are the
    /// other bent's (`columns` is empty here).
    pub carried: bool,
}

/// What `StructureWorld::build` builds. With eo-app's `--structures` off no
/// world is built at all (consumers hold `None`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructureOptions {
    /// Tunnel bores and portals (`--tunnels`), only with `--structures`.
    pub tunnels: bool,
}

/// The road class a chain is solved by (its grade limit and the clearance
/// it asks of what passes over it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RoadClass {
    Motorway,
    MotorwayLink,
    Trunk,
    Other,
}

impl RoadClass {
    pub const ALL: [RoadClass; 4] = [RoadClass::Motorway, RoadClass::MotorwayLink, RoadClass::Trunk, RoadClass::Other];

    pub fn of(highway: &str) -> Self {
        match highway {
            "motorway" => RoadClass::Motorway,
            "motorway_link" => RoadClass::MotorwayLink,
            "trunk" => RoadClass::Trunk,
            _ => RoadClass::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RoadClass::Motorway => "motorway",
            RoadClass::MotorwayLink => "motorway_link",
            RoadClass::Trunk => "trunk",
            RoadClass::Other => "other",
        }
    }
}

/// Whether a structure over a way of this OSM class must give it the freeway
/// clearance (`structure-vertical-clearance-freeway`: "a freeway or
/// expressway roadbed, including main lanes, shoulders and ramps"); every
/// other way gets `structure-vertical-clearance-local`.
pub fn freeway_clearance(highway: &str) -> bool {
    matches!(highway, "motorway" | "motorway_link" | "trunk" | "trunk_link")
}

/// What a chain is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChainKind {
    /// Above grade (a bridge, or a positive OSM layer): a deck on bents.
    Deck,
    /// An open road below other ways (a negative layer, not a tunnel): it
    /// follows the ground and passes under what crosses it.
    Below,
    /// A tunnel (`tunnel` tag): under the ground between its portals.
    Tunnel,
    /// Carried by a landmark bridge: its profile is the landmark's own deck;
    /// stage A2 builds collision for it but no render mesh (the landmark
    /// draws it).
    Landmark,
}

pub use eo_landmarks::bridge_roads::LandmarkDeck;

/// What a chain's end meets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EndKind {
    /// A paved street (a run end or a run's middle within the snap): the end
    /// is held at the street datum, `ground + roadLift`, there.
    Paved,
    /// A landmark deck's end: the end is held at the landmark's deck height.
    Landmark,
    /// Other chains: all of them meet at one height (solved).
    Shared,
    /// Nothing: the end is wherever the profile's own bounds put it.
    Free,
}

impl EndKind {
    pub fn name(self) -> &'static str {
        match self {
            EndKind::Paved => "paved",
            EndKind::Landmark => "landmark",
            EndKind::Shared => "shared",
            EndKind::Free => "free",
        }
    }
}

/// One end of a chain.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainEnd {
    pub kind: EndKind,
    /// The node the end is at (shared by every chain end there).
    pub node: u32,
    /// The chain's own end point (where a paved end takes the street datum).
    pub at: [f64; 2],
    /// The height the end must meet: the street as drawn at the end point
    /// less the deck's own crown (paved; the street datum where nothing is
    /// drawn), the landmark deck (landmark), the solved node height (shared,
    /// when built); None for a free end.
    pub fixed: Option<f64>,
    /// A paved end's street surface just past the end, across the deck:
    /// (lateral offset, height over `fixed`) at the deck top's points (the
    /// meshes blend the deck top to it; empty for other ends).
    pub surface: Vec<[f64; 2]>,
}

/// Why a chain was not built. Nothing is invented to rescue one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hold {
    /// A fixed end is lower than the clearance over `over` reaches back to it
    /// along the grade limit.
    EndTooLow { over: String },
    /// The ends (or the ground, or a shared end another chain sets) cannot
    /// be met within the grade limit.
    Grade,
    /// The standards table has no verified clearance for what it crosses,
    /// or for the open water it spans (a navigation clearance, none in the
    /// table): a deck over open water for longer than a creek crossing
    /// (`solve.rs` `OPEN_WATER_HOLD_M`).
    NoSourcedClearance,
    /// It crosses a way of its own level with no shared node: which one is
    /// on top is not known.
    UnknownCrossing { with: String },
    /// Paving or a lower roadway leaves no room for a bent over a stretch
    /// longer than `LONGEST_SPAN_M` (a street running along under the deck).
    NoBentRoom { span: u32 },
}

impl Hold {
    /// The reason without its subject, for counting.
    pub fn code(&self) -> &'static str {
        match self {
            Hold::EndTooLow { .. } => "end too low for clearance",
            Hold::Grade => "grade",
            Hold::NoSourcedClearance => "no sourced clearance",
            Hold::UnknownCrossing { .. } => "unknown crossing",
            Hold::NoBentRoom { .. } => "no bent room",
        }
    }
}

impl fmt::Display for Hold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hold::EndTooLow { over } => write!(f, "end too low for clearance over {over}"),
            Hold::Grade => write!(f, "grade"),
            Hold::NoSourcedClearance => write!(f, "no sourced clearance"),
            Hold::UnknownCrossing { with } => write!(f, "unknown crossing with {with}"),
            Hold::NoBentRoom { span } => write!(f, "no bent room: a {span} m span (longest allowed {LONGEST_SPAN_M} m)"),
        }
    }
}

/// Whether a chain is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Built,
    Held(Hold),
    /// A tunnel solved but not built: `--tunnels` is off.
    TunnelOff,
}

/// One street-plan segment (or the part of one a landmark deck end splits)
/// along a chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Index into the street world's `city.segments`.
    pub segment: usize,
    /// The segment's stable key (`sf-street-<way>:<n>`), with `#<part>` when
    /// a landmark deck end splits it.
    pub key: String,
    /// OSM way id.
    pub way: String,
    pub name: String,
    pub highway: String,
    /// Half the section width (the segment's `width`, or its lane layout).
    pub half: f64,
    /// Its stretch of the chain (stations).
    pub s0: f64,
    pub s1: f64,
    /// It runs against its segment's own point order.
    pub reversed: bool,
}

/// Something a deck passes over, and what the solve gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Crossing {
    /// What is crossed: a street (`<name> (<run id>)`) or a lower chain.
    pub over: String,
    /// The lower chain, None for a paved street.
    pub chain: Option<u32>,
    /// `structure-vertical-clearance-freeway` applies (else `-local`).
    pub freeway: bool,
    /// Stations over it (the deck within the crossed carriageway's half
    /// width plus its own).
    pub s0: f64,
    pub s1: f64,
    /// The table's clearance (NaN when none is sourced).
    pub clearance: f64,
    /// The least clearance under the superstructure over [s0, s1] after the
    /// solve: deck top minus depth minus the crossed surface.
    pub achieved: f64,
}

/// A grade-separated road between two nodes, with its profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub id: u32,
    /// OSM `layer` (a bridge without one is 1, a tunnel without one -1).
    pub level: i8,
    pub class: RoadClass,
    pub kind: ChainKind,
    pub landmark: Option<LandmarkDeck>,
    /// Centreline (plan, ground-scaled) and its arc table (`eo_street::plan::arc_table`).
    pub points: Vec<[f64; 2]>,
    pub cum: Vec<f64>,
    /// The segments along it, in order.
    pub pieces: Vec<Piece>,
    pub ends: [ChainEnd; 2],
    /// Profile stations (at most `STATION_MAX_M` apart, evenly spaced, from
    /// 0 to the length) and the deck top at each, game frame.
    pub stations: Vec<f64>,
    pub heights: Vec<f64>,
    /// The grade limit over each step between stations.
    pub grade_limits: Vec<f64>,
    /// Bent stations (deck chains), in order; the spans run between the ends
    /// and the bents.
    pub bents: Vec<f64>,
    /// Where each bent's columns may stand: the stretches across the deck
    /// (lateral offsets along the frame normal `(nx, nz)`, from -(half +
    /// `BENT_FOOTPRINT_MARGIN_M`) to +(half + margin): a column may stand up
    /// to its own radius past the deck edge, under the cap's overhang; a
    /// straddle bent's reach `OUTRIGGER_REACH_M` further) where a column
    /// footprint (`BENT_FOOTPRINT_MARGIN_M`) is clear of any lower chain's
    /// roadway and either off paving or, by the bent's rule, wholly in a
    /// street's median, parking lane or kerb-side furniture zone; never
    /// empty for a placed bent, except one carried by a twin deck's cap
    /// (`Bent::twin`), whose room is the twin's (a shared bent's zones span
    /// both decks, along its own line). The cap spans the rest.
    pub bent_columns: Vec<Vec<[f64; 2]>>,
    /// Each bent's rule, columns and cap (built deck chains; one per `bents`
    /// entry, empty otherwise). `bent_columns` keeps the room each bent
    /// had; these are the columns stood in it.
    pub supports: Vec<Bent>,
    /// Superstructure depth of each span (`bents.len() + 1` of them; empty
    /// when there is no superstructure).
    pub depths: Vec<f64>,
    /// What it passes over.
    pub crossings: Vec<Crossing>,
    pub status: Status,
    /// Built same-level decks this one overlaps and is solved to meet
    /// (stage A5): a ramp's gore with its mainline, twin carriageways side
    /// by side. Where their roadways overlap the two profiles are one
    /// surface, and the meshes draw it once.
    pub merged_with: Vec<u32>,
    /// A deck held by clearance or grade: the least height
    /// each fixed end would need to meet the crossing, terrain and opposite end within
    /// its grade (the binding bound's reach back to the end); stage A6 asks a
    /// paved stub to climb to it (an approach embankment,
    /// `StructureWorld::street_joins`). None elsewhere.
    pub end_wants: [Option<f64>; 2],
}

impl Chain {
    pub(crate) fn shares_end(&self, other: &Chain) -> bool {
        self.ends.iter().any(|end| other.ends.iter().any(|n| n.node == end.node))
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Built (solved and kept, or a landmark's deck).
    pub fn is_built(&self) -> bool {
        self.status == Status::Built
    }

    pub fn hold(&self) -> Option<&Hold> {
        match &self.status {
            Status::Held(h) => Some(h),
            _ => None,
        }
    }

    /// The centreline frame at station `s` (`eo_street::plan::frame_at`).
    pub fn frame(&self, s: f64) -> Frame {
        frame_at(&self.points, &self.cum, s.clamp(0.0, self.length()), false)
    }

    /// Index of the station step holding `s`.
    fn step(&self, s: f64) -> usize {
        let n = self.stations.len();
        if n < 2 {
            return 0;
        }
        match self.stations.binary_search_by(|t| t.total_cmp(&s)) {
            Ok(i) => i.min(n - 2),
            Err(0) => 0,
            Err(i) => (i - 1).min(n - 2),
        }
    }

    /// Deck top at station `s` (linear between stations).
    pub fn y_at(&self, s: f64) -> f64 {
        let n = self.stations.len();
        if n == 0 {
            return 0.0;
        }
        if n == 1 {
            return self.heights[0];
        }
        let i = self.step(s);
        let (s0, s1) = (self.stations[i], self.stations[i + 1]);
        let t = if s1 > s0 { ((s - s0) / (s1 - s0)).clamp(0.0, 1.0) } else { 0.0 };
        self.heights[i] + (self.heights[i + 1] - self.heights[i]) * t
    }

    /// Rise over run along increasing station at `s`.
    pub fn slope_at(&self, s: f64) -> f64 {
        if self.stations.len() < 2 {
            return 0.0;
        }
        let i = self.step(s);
        let ds = self.stations[i + 1] - self.stations[i];
        if ds > 0.0 { (self.heights[i + 1] - self.heights[i]) / ds } else { 0.0 }
    }

    /// The piece at station `s`.
    pub fn piece_at(&self, s: f64) -> &Piece {
        self.pieces.iter().find(|p| s <= p.s1 + 1e-9).unwrap_or_else(|| self.pieces.last().expect("a chain has pieces"))
    }

    /// Half the section width at station `s`.
    pub fn half_at(&self, s: f64) -> f64 {
        self.piece_at(s).half
    }

    /// The widest half section along it.
    pub fn max_half(&self) -> f64 {
        self.pieces.iter().map(|p| p.half).fold(0.0, f64::max)
    }

    /// The span holding station `s`: (start, end).
    pub fn span_at(&self, s: f64) -> (f64, f64) {
        let mut a = 0.0;
        for &b in &self.bents {
            if s < b {
                return (a, b);
            }
            a = b;
        }
        (a, self.length())
    }

    /// Superstructure depth at station `s` (0 without a superstructure).
    pub fn depth_at(&self, s: f64) -> f64 {
        if self.depths.is_empty() {
            return 0.0;
        }
        let k = self.bents.iter().take_while(|&&b| s >= b).count();
        self.depths[k.min(self.depths.len() - 1)]
    }

    /// Its OSM way ids, sorted, once each.
    pub fn ways(&self) -> Vec<&str> {
        let mut w: Vec<&str> = self.pieces.iter().map(|p| p.way.as_str()).collect();
        w.sort();
        w.dedup();
        w
    }

    /// The first name along it ("" when none).
    pub fn name(&self) -> &str {
        self.pieces.iter().map(|p| p.name.as_str()).find(|n| !n.is_empty()).unwrap_or("")
    }

    /// The nearest point of the centreline to (x, z): (station, distance).
    pub fn project(&self, x: f64, z: f64) -> (f64, f64) {
        let mut best = (0.0, f64::INFINITY);
        for i in 0..self.points.len().saturating_sub(1) {
            let (t, d, _) = index::project([x, z], self.points[i], self.points[i + 1]);
            if d < best.1 {
                best = (self.cum[i] + t * (self.cum[i + 1] - self.cum[i]), d);
            }
        }
        best
    }

    /// The deck at station `s`.
    pub fn deck_point(&self, s: f64) -> DeckPoint {
        let f = self.frame(s);
        DeckPoint { y: self.y_at(s), slope: self.slope_at(s), chain: self.id, s, dir: [f.tx, f.tz] }
    }
}

/// The grade-separated roads of the city: chains, profiles, bents and meshes
/// (meshes are filled by later stages). Shared read-only between threads (`Arc`).
#[derive(Debug, Default)]
pub struct StructureWorld {
    options: StructureOptions,
    chains: Vec<Chain>,
    /// `deck_at`'s index: (level, cell x, cell z) -> (chain, centreline segment).
    decks: HashMap<(i8, i32, i32), Vec<(u32, u32)>>,
    /// Levels with a built deck, ascending.
    deck_levels: Vec<i8>,
    /// OSM way id -> level, for the ways a built chain carries.
    levels: HashMap<String, i8>,
    landmark_lanes: HashMap<String, Vec<eo_data::sim_source::TrafficDeckLane>>,
    census: Census,
    /// The meshes by tile (filled by `attach_meshes`).
    tiles: tiles::Tiles,
    /// Column footprints by tile.
    columns: std::collections::BTreeMap<(i32, i32), Vec<ColumnFootprint>>,
    /// The clearance envelope's index (stage A6; filled by `attach_meshes`).
    clear: clear::ClearIndex,
    /// `concrete-barrier-type-60m-base-width` (what a deck's end face closes
    /// past its roadway, `street_joins`).
    barrier_base: f64,
    /// Stage A7 (`--tunnels`): the lights along the bores by tile, and the
    /// ground the tunnels' open stretches cut through, gridded.
    lights: std::collections::BTreeMap<(i32, i32), Vec<TunnelLight>>,
    cuts: Vec<TerrainCut>,
    cut_grid: HashMap<(i32, i32), Vec<u32>>,
    /// The bores (covered stretches), gridded (`bore_at`).
    bores: Vec<Bore>,
    bore_grid: HashMap<(i32, i32), Vec<u32>>,
}

/// One covered stretch of a tunnel (stage A8, `--tunnels`): the bore under
/// the hill, where neither the sun nor the sky reach (`StructureWorld::bore_at`).
#[derive(Clone, Debug, PartialEq)]
pub struct Bore {
    pub chain: u32,
    /// Measured inner section of the stacked Yerba Buena traffic decks, when enabled.
    pub island_section: Option<IslandTunnelSection>,
    /// The stations of its first and last covered station.
    pub s0: f64,
    pub s1: f64,
    /// Whether its start and its end are portals (a covered station beside
    /// an open one, where daylight comes in); a covered chain end is not.
    pub portal: [bool; 2],
    /// Its stations, in order.
    pub points: Vec<BorePoint>,
    /// Its bounds (plan).
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// One station of a bore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorePoint {
    pub x: f64,
    pub z: f64,
    pub s: f64,
    /// The roadway's centreline height.
    pub y: f64,
    /// The world offset per metre of lateral offset (+n, mitred).
    pub nx: f64,
    pub nz: f64,
    /// The inside's reach to the walls' backs (+n, -n; lateral offset).
    pub reach: [f64; 2],
}

/// A point inside a bore (`StructureWorld::bore_at`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoreHit {
    pub chain: u32,
    pub s: f64,
    /// How far in along the bore from its nearest portal (infinite when
    /// neither end is a portal).
    pub depth: f64,
    /// Height over the roadway's centreline there.
    pub above: f64,
    /// The nearer portal's face (its centreline point, plan) and the unit
    /// plan direction into the bore from it: [x, z, dx, dz]; None when
    /// neither end is a portal.
    pub portal: Option<[f64; 4]>,
}

/// A bore holds the points from this far under its roadway (its cut floor
/// and a margin) up to this far over it (the vault's crown and its roof,
/// `tunnel::ROOF_M`, and a margin under the portal canopy's top, which stands
/// a parapet over the roof at least). INFERRED (drawing reaches).
pub const BORE_BELOW_M: f64 = 1.5;
pub const BORE_ABOVE_M: f64 = BORE_CLEAR_M + tunnel::ROOF_M + 0.3;

impl Bore {
    /// (x, y, z) inside it: (station, lateral offset, height over the roadway), else None.
    pub fn locate(&self, x: f64, y: f64, z: f64) -> Option<(f64, f64, f64)> {
        if x < self.min[0] || x > self.max[0] || z < self.min[1] || z > self.max[1] {
            return None;
        }
        let mut best: Option<(f64, f64, f64, f64)> = None; // (plan distance, s, u, above)
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let l2 = dx * dx + dz * dz;
            if l2 < 1e-12 {
                continue;
            }
            let t = ((x - a.x) * dx + (z - a.z) * dz) / l2;
            if !(-1e-6..=1.0 + 1e-6).contains(&t) {
                continue;
            }
            let t = t.clamp(0.0, 1.0);
            let (qx, qz) = (a.x + dx * t, a.z + dz * t);
            let (ox, oz) = (a.nx + (b.nx - a.nx) * t, a.nz + (b.nz - a.nz) * t);
            let o2 = (ox * ox + oz * oz).max(1e-12);
            let u = ((x - qx) * ox + (z - qz) * oz) / o2;
            let reach = if u >= 0.0 { a.reach[0] + (b.reach[0] - a.reach[0]) * t } else { a.reach[1] + (b.reach[1] - a.reach[1]) * t };
            if u.abs() > reach {
                continue;
            }
            let above = y - (a.y + (b.y - a.y) * t);
            let ceiling = self.island_section.map_or(BORE_ABOVE_M, |section| {
                section.height(u / reach) + (BORE_ABOVE_M - BORE_CLEAR_M)
            });
            if !(-BORE_BELOW_M..=ceiling).contains(&above) {
                continue;
            }
            let d = (x - qx).hypot(z - qz);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, a.s + (b.s - a.s) * t, u, above));
            }
        }
        best.map(|b| (b.1, b.2, b.3))
    }

    /// How far station `s` is from its nearest portal (infinite with none).
    pub fn depth(&self, s: f64) -> f64 {
        let a = if self.portal[0] { s - self.s0 } else { f64::INFINITY };
        let b = if self.portal[1] { self.s1 - s } else { f64::INFINITY };
        a.min(b).max(0.0)
    }

    /// The portal nearer station `s`: its face's centreline point and the
    /// unit plan direction into the bore ([x, z, dx, dz]); None with none.
    pub fn portal_at(&self, s: f64) -> Option<[f64; 4]> {
        let n = self.points.len();
        if n < 2 {
            return None;
        }
        let a = if self.portal[0] { s - self.s0 } else { f64::INFINITY };
        let b = if self.portal[1] { self.s1 - s } else { f64::INFINITY };
        if a.is_infinite() && b.is_infinite() {
            return None;
        }
        let (p, q) = if a <= b { (self.points[0], self.points[1]) } else { (self.points[n - 1], self.points[n - 2]) };
        let (dx, dz) = (q.x - p.x, q.z - p.z);
        let l = dx.hypot(dz).max(1e-9);
        Some([p.x, p.z, dx / l, dz / l])
    }
}

/// One light along a tunnel bore (stage A7): under the crown, shining down
/// on the roadway (`StructureWorld::tunnel_lights`; eo-props hands them to
/// the renderer's light list as the props' lamps).
#[derive(Clone, Debug, PartialEq)]
pub struct TunnelLight {
    pub chain: u32,
    /// Its station along the chain.
    pub s: f64,
    /// Its position (world metres, game frame).
    pub pos: [f64; 3],
    /// Half the bore's inside width there (roadway and walkways), and the
    /// roadway's height under it.
    pub half_width: f64,
    pub floor: f64,
    /// Fraction of the longitudinal station's light emitted by this aperture.
    pub flux_share: f32,
}

/// The ground one open stretch of a tunnel cuts through (stage A7): the
/// approach cut between the street and a portal, or a whole underpass the
/// ground never covers. The terrain and the collision heightfield leave
/// this out (or lower it to `floor`) only with `--tunnels`; the tunnel's
/// retaining walls and portal headwall stand on its edges.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainCut {
    pub chain: u32,
    /// The stretch of the chain it covers (stations), portal faces included.
    pub s0: f64,
    pub s1: f64,
    /// The footprint (plan, world metres): one wall back along the chain,
    /// then the other back, a simple polygon.
    pub ring: Vec<[f64; 2]>,
    /// The cut's floor: under the lowest roadway of the stretch by
    /// `tunnel::CUT_BELOW_M` (what a lowered terrain vertex inside goes to).
    pub floor: f64,
    /// Its bounds.
    pub min: [f64; 2],
    pub max: [f64; 2],
    /// A portal's canopy (`tunnel::PORTAL_OPEN_M` back over the bore from
    /// its headwall, whose top and ends reach back over it): the terrain's
    /// cells meeting it are left out, not lowered (a lowered terrain would
    /// slope into the bore behind the face). An open cut's ring (`false`):
    /// the terrain inside it, and one cell round it, is lowered to `floor`.
    pub canopy: bool,
}

impl TerrainCut {
    /// (x, z) is inside the footprint.
    pub fn contains(&self, x: f64, z: f64) -> bool {
        if x < self.min[0] || x > self.max[0] || z < self.min[1] || z > self.max[1] {
            return false;
        }
        eo_street::point_in_ring(x, z, &self.ring)
    }
}

/// Collision triangles of one tile, in world metres (as eo-player's
/// `SurfaceMesh`): deck tops are walkable street, everything else (barriers,
/// columns, abutments, portal walls) blocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructureSurface {
    pub street: Vec<[[f64; 3]; 3]>,
    pub barrier: Vec<[[f64; 3]; 3]>,
}

/// The deck under a lane at one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckPoint {
    /// Height of the deck's driven surface, game frame.
    pub y: f64,
    /// Rise over run along `dir` (the chain's increasing station); a lane
    /// heading `h` (unit, plan) climbs `slope * (dir . h)` (`slope_toward`).
    pub slope: f64,
    /// The chain and its station there.
    pub chain: u32,
    pub s: f64,
    /// Unit plan direction of increasing station.
    pub dir: [f64; 2],
}

impl DeckPoint {
    /// Rise over run for travel along the unit plan heading `heading`.
    pub fn slope_toward(&self, heading: [f64; 2]) -> f64 {
        self.slope * (self.dir[0] * heading[0] + self.dir[1] * heading[1])
    }
}

/// What a vertex of a `StructureMesh` is made of; A3 maps each to a
/// material. `classes` holds these as `u8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum StructureClass {
    /// The driven deck: the street's asphalt material; RGB is the street's
    /// own carriageway tint for that segment (`StreetMaterial::carriageway_tint`).
    DeckSurface = 0,
    /// Lane and edge lines: the vertex colour (the street palette's marking
    /// white or yellow), lifted `PAINT_LIFT_M` over the deck.
    Paint = 1,
    /// Barriers, fascia, deck overhang, webs, soffit, cap beams, capital
    /// blocks, columns, abutment, fill and end walls: concrete; RGB is the
    /// concrete tint, alpha its wear.
    Concrete = 2,
    /// Expansion joint strips across the deck (steel armour and seal).
    Joint = 3,
    /// Drain downspouts (painted steel pipe).
    Metal = 4,
}

impl StructureClass {
    pub const ALL: [StructureClass; 5] = [StructureClass::DeckSurface, StructureClass::Paint, StructureClass::Concrete, StructureClass::Joint, StructureClass::Metal];

    pub fn of(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
}

/// Paint stands this far above the deck surface (`StructureClass::Paint`).
pub const PAINT_LIFT_M: f64 = mesh::PAINT_LIFT_M;

/// Render geometry of one tile at one level of detail, as plain arrays the
/// renderer uploads like `eo_street::TileMesh` (positions in world metres).
/// The vertex encoding (tint and wear in `colors`, formwork coordinates in
/// `uv`) is documented on the mesh module and in `StructureClass`; in short:
/// alpha = wear (0 clean .. 255 stained) on `Concrete`, `Joint`, `Metal`,
/// 255 elsewhere except for `EAST_SPAN_PAVING_ALPHA` on a registered deck; `uv` = (along the boards, across them) in metres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructureMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// RGBA8 per vertex (`eo_street::mesh::rgba` packing): tint, and wear in alpha.
    pub colors: Vec<u32>,
    /// `StructureClass` per vertex.
    pub classes: Vec<u8>,
    /// Surface coordinates per vertex, metres: `[0]` along the formwork
    /// boards (the chain's station; a column's height above its base),
    /// `[1]` across them (the lateral offset on near-horizontal faces, the
    /// height relative to the deck top on near-vertical faces, the distance
    /// round a column; on faces across the chain (lateral offset, height)).
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Stage A9: the `Era` of the structure each vertex belongs to (`Era as
    /// u8`), how weathered the renderer draws its concrete with `--grime`
    /// (nothing else reads it). Empty when built elsewhere: every vertex
    /// `Era::Old`.
    pub eras: Vec<u8>,
}

impl StructureMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Bytes of its arrays (as stored here).
    pub fn bytes(&self) -> usize {
        self.positions.len() * 12 + self.normals.len() * 12 + self.colors.len() * 4 + self.classes.len() + self.uv.len() * 8 + self.indices.len() * 4 + self.eras.len()
    }

    /// The era of vertex `i` (`Era::Old` where none is recorded).
    pub fn era(&self, i: usize) -> Era {
        Era::of(self.eras.get(i).copied().unwrap_or(0))
    }
}

/// How long a structure's concrete has stood in the weather (stage A9): the
/// renderer draws its run-off streaks, efflorescence, rust and soiling this
/// heavily with `--grime` (eo-render structures.wgsl); nothing else reads it.
/// From the build years the reference sheet records (section 1.1, the FHWA
/// National Bridge Inventory; 1.2 and 1.7): the city's freeway viaducts and
/// ramps of 1955-1971 (NBI 34 0077 and 34 0088 US-101, 1955; 34 0100 I-280
/// China Basin, 1971; 34 0026 I-280 Alemany, 1966/72; the Bayshore I-80
/// viaduct, about 1955) and the Broadway (1952), MacArthur (1940) and
/// Stockton (1914) tunnels are old; the Central Viaduct (NBI 34 0177, 2005)
/// and the Bay Bridge West Approach with its ramps (NBI 34 0126L/R, rebuilt
/// 2008-09) are rebuilt; the Presidio Parkway's viaducts and tunnels
/// (2012-15) are new.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Era {
    Old = 0,
    Rebuilt = 1,
    New = 2,
}

impl Era {
    pub fn of(v: u8) -> Self {
        match v {
            1 => Era::Rebuilt,
            2 => Era::New,
            _ => Era::Old,
        }
    }
}

/// The rebuilt Bay Bridge West Approach reaches this far from the bridge
/// (real metres; times the ground scale in the game's plan): "about 1 mile",
/// 5th Street to the Beale Street anchorage, with six ramps (reference sheet
/// 1.2, Bay Bridge Public Information Office, West Approach Fact Sheet).
pub const WEST_APPROACH_M: f64 = 1609.344;

/// A chain's era (`Era`): by its name (the Presidio Parkway new, the Central
/// Freeway's 2005 viaduct rebuilt), else rebuilt within `WEST_APPROACH_M`
/// (ground-scaled) of a Bay Bridge deck's end (the West Approach and its
/// ramps), else old. `chains`: every chain (the landmark decks' ends).
pub fn era_of(c: &Chain, chains: &[Chain], scale: f64) -> Era {
    let name = c.name();
    if name.contains("Presidio Parkway") || east_span::readings(c).is_some() {
        return Era::New;
    }
    if name.contains("Central Freeway") {
        return Era::Rebuilt;
    }
    let reach = WEST_APPROACH_M * scale;
    let anchors: Vec<[f64; 2]> = chains
        .iter()
        .filter(|b| matches!(b.landmark, Some(LandmarkDeck::BayBridgeUpper | LandmarkDeck::BayBridgeLower)))
        .flat_map(|b| [b.points.first().copied(), b.points.last().copied()])
        .flatten()
        .collect();
    let near = c.points.iter().any(|p| anchors.iter().any(|a| (p[0] - a[0]).hypot(p[1] - a[1]) <= reach));
    if near { Era::Rebuilt } else { Era::Old }
}

/// One column's footprint, for keeping cars, trees, lamps and furniture
/// off it (A6).
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnFootprint {
    pub chain: u32,
    /// Index into the chain's `bents` / `supports`.
    pub bent: u32,
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub site: ColumnSite,
    pub street: Option<StreetRef>,
}

/// What the meshes of every tile add up to (`StructureWorld::mesh_stats`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshStats {
    pub tiles: usize,
    /// Per level (near, far, coarse).
    pub triangles: [usize; 3],
    pub vertices: [usize; 3],
    pub bytes: [usize; 3],
    /// The largest single tile per level: triangles, vertices.
    pub max_tile_triangles: [usize; 3],
    pub max_tile_vertices: [usize; 3],
    /// Collision triangles: deck surface (street), concrete (barrier).
    pub street_triangles: usize,
    pub barrier_triangles: usize,
}

/// The tags of one OSM way the solve reads (from `eo_data::Road`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoadTags {
    pub layer: i32,
    pub bridge: bool,
    pub tunnel: bool,
    pub name: String,
}

fn road_tags(city: &City) -> HashMap<String, RoadTags> {
    city.roads.iter().map(|r| (r.id.clone(), RoadTags { layer: r.layer, bridge: r.bridge, tunnel: r.tunnel, name: r.name.clone() })).collect()
}

/// The way id a lane or a street segment names: `"<id>"` or `"sf-street-<id>"`.
fn way_key(way_id: &str) -> &str {
    way_id.strip_prefix("sf-street-").unwrap_or(way_id)
}

impl StructureWorld {
    /// The city's landmark bridge geometry is already installed in its collision tiles.
    pub fn has_landmark_roads(&self) -> bool {
        self.tiles.has_landmark_roads()
    }

    /// A world with nothing in it.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds from the street world (its city, plan, terrain and options:
    /// the separated segments, the paved network, the street datum), the
    /// city's roads (`layer`, `bridge`, `tunnel` by way id), the standards
    /// table and the landmark bridges (the ways they carry, their decks).
    /// Every input is one eo-app already holds when the street world is
    /// ready; the elevation grid is `streets.terrain.grid`.
    ///
    /// Errors only when the table lacks a grade limit or the depth ratio
    /// (a missing clearance holds the chains that need it instead).
    pub fn build(streets: &StreetWorld, city: &City, standards: &CaStandards, landmarks: &LandmarksJs, options: StructureOptions) -> anyhow::Result<Self> {
        let mut w = Self::solve_only(streets, city, standards, landmarks, options)?;
        w.attach(streets, city, standards, landmarks)?;
        Ok(w)
    }

    /// `build` without the meshes (chains, profiles, bents, holds, census):
    /// what stage A6's first passes need (`build_joined`).
    pub(crate) fn solve_only(streets: &StreetWorld, city: &City, standards: &CaStandards, landmarks: &LandmarksJs, options: StructureOptions) -> anyhow::Result<Self> {
        let roads = road_tags(city);
        let specs = landmark::specs(landmarks, &streets.terrain.grid, &roads);
        solve::build(streets, &roads, standards, specs, options)
    }

    /// The meshes of a solved world (`build`'s second half).
    pub(crate) fn attach(&mut self, streets: &StreetWorld, city: &City, standards: &CaStandards, landmarks: &LandmarksJs) -> anyhow::Result<()> {
        // the class-model lane width of each way a fallback section needs.
        let by_id: HashMap<&str, &eo_data::Road> = city.roads.iter().map(|r| (r.id.as_str(), r)).collect();
        let state = standards.metric("travel-lane-width-state-highway-min").ok();
        let lane = |way: &str| -> f64 { by_id.get(way).and_then(|r| standards.class_lane_width(r).ok()).or(state).unwrap_or(0.0) };
        let apron = landmark::marin_apron_chains(landmarks, &self.chains);
        self.attach_meshes_with_apron(streets, standards, &lane, &apron)?;
        let section = eo_landmarks::suspension::DeckStandards::load(standards)?;
        let roads = eo_landmarks::bridge_roads::surfaces(landmarks, &|x, z| streets.terrain.grid.sample(x, z), &section);
        self.tiles.install_landmark_roads(&self.chains, roads, &mut self.clear);
        self.landmark_lanes = landmark::golden_gate_traffic_lanes(landmarks, streets, &section, &by_id, self);
        Ok(())
    }

    /// Sweeps every built chain into its tiles (the meshes themselves are
    /// built when a tile is first asked for) and indexes the columns.
    #[cfg(test)]
    pub(crate) fn attach_meshes(&mut self, streets: &StreetWorld, standards: &CaStandards, class_lane: &dyn Fn(&str) -> f64) -> anyhow::Result<()> {
        self.attach_meshes_with_apron(streets, standards, class_lane, &[])
    }

    fn attach_meshes_with_apron(&mut self, streets: &StreetWorld, standards: &CaStandards, class_lane: &dyn Fn(&str) -> f64, apron: &[u32]) -> anyhow::Result<()> {
        let dims = section::Dims::read(standards)?;
        self.tiles = tiles::Tiles::build(streets, &self.chains, dims, class_lane, apron);
        self.census.section_fallbacks = self.tiles.fallback_chains;
        self.columns.clear();
        for c in self.chains.iter().filter(|c| c.is_built() && c.kind == ChainKind::Deck) {
            for (k, b) in c.supports.iter().enumerate() {
                for col in &b.columns {
                    let f = ColumnFootprint { chain: c.id, bent: k as u32, x: col.x, z: col.z, radius: col.radius, site: col.site, street: col.street.clone() };
                    self.columns.entry(eo_street::tile_key(col.x, col.z)).or_default().push(f);
                }
            }
        }
        self.clear = clear::ClearIndex::build(&self.chains, &self.tiles.sweeps, dims.barrier_h, dims.barrier_base);
        // stage A9: retaining walls where the joined street meets the structures.
        let walls = {
            let (chains, sweeps) = (&self.chains, &self.tiles.sweeps);
            let joins = streets.o.structures.clone();
            let structure = |j: usize| -> Option<(u8, [u8; 3])> {
                let seg = joins.as_ref()?.ends.get(j)?.segment;
                let ci = chains.iter().position(|c| c.is_built() && c.pieces.iter().any(|p| p.segment == seg))?;
                let sw = sweeps.get(ci)?.as_ref()?;
                Some((sw.era, sw.tint))
            };
            let mut w = walls::stub_walls(streets, &dims, &structure);
            let blocked = |x: f64, z: f64, r: f64, y0: f64, y1: f64| self.blocked(x, z, r, y0, y1);
            w.extend(walls::leg_walls(streets, &blocked, &structure));
            w
        };
        self.census.walls = walls.len();
        self.tiles.add_walls(walls);
        // stage A7: the bores' lights and the tunnels' cuts.
        self.lights.clear();
        self.cuts.clear();
        self.cut_grid.clear();
        self.bores.clear();
        self.bore_grid.clear();
        for (c, sw) in self.chains.iter().zip(&self.tiles.sweeps) {
            let Some(sw) = sw.as_ref().filter(|s| s.draw == sweep::Draw::Tunnel) else { continue };
            for l in tunnel::lights(c, sw) {
                self.lights.entry(eo_street::tile_key(l.pos[0], l.pos[2])).or_default().push(l);
            }
            self.cuts.extend(tunnel::cuts(c, sw));
            self.bores.extend(tunnel::bores(c, sw));
        }
        for (i, cut) in self.cuts.iter().enumerate() {
            for gz in index::cell_of(cut.min[1])..=index::cell_of(cut.max[1]) {
                for gx in index::cell_of(cut.min[0])..=index::cell_of(cut.max[0]) {
                    self.cut_grid.entry((gx, gz)).or_default().push(i as u32);
                }
            }
        }
        for (i, b) in self.bores.iter().enumerate() {
            for gz in index::cell_of(b.min[1])..=index::cell_of(b.max[1]) {
                for gx in index::cell_of(b.min[0])..=index::cell_of(b.max[0]) {
                    self.bore_grid.entry((gx, gz)).or_default().push(i as u32);
                }
            }
        }
        Ok(())
    }

    /// Every bore (the tunnels' covered stretches; stage A8, none without `--tunnels`), by chain then station.
    pub fn bores(&self) -> &[Bore] {
        &self.bores
    }

    /// The bores whose bounds meet the rectangle `min..max` (plan, world metres).
    pub fn bores_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<&Bore> {
        let mut ids: Vec<u32> = vec![];
        for gz in index::cell_of(min[1])..=index::cell_of(max[1]) {
            for gx in index::cell_of(min[0])..=index::cell_of(max[0]) {
                if let Some(v) = self.bore_grid.get(&(gx, gz)) {
                    ids.extend(v);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().map(|i| &self.bores[i as usize]).filter(|b| b.max[0] >= min[0] && b.min[0] <= max[0] && b.max[1] >= min[1] && b.min[1] <= max[1]).collect()
    }

    /// The bore holding (x, y, z) (inside its walls' backs, from its cut
    /// floor to over its roof; stage A8, `--tunnels`): its chain, station,
    /// how far in from the nearest portal and the height over its roadway;
    /// the deepest where bores overlap. None outside every bore.
    pub fn bore_at(&self, x: f64, y: f64, z: f64) -> Option<BoreHit> {
        let ids = self.bore_grid.get(&(index::cell_of(x), index::cell_of(z)))?;
        ids.iter()
            .filter_map(|&i| {
                let b = &self.bores[i as usize];
                b.locate(x, y, z).map(|(s, _, above)| BoreHit { chain: b.chain, s, depth: b.depth(s), above, portal: b.portal_at(s) })
            })
            .max_by(|a, b| a.depth.total_cmp(&b.depth))
    }

    /// The lights along the tunnel bores in one 128 m tile (stage A7; none
    /// without `--tunnels`), by chain and station.
    pub fn tunnel_lights(&self, key: (i32, i32)) -> Vec<TunnelLight> {
        self.lights.get(&key).cloned().unwrap_or_default()
    }

    /// Every tunnel light, by tile then chain and station.
    pub fn all_tunnel_lights(&self) -> Vec<TunnelLight> {
        self.lights.values().flatten().cloned().collect()
    }

    /// The same swept traffic space used to clip the tunnel linings and street surfaces.
    pub fn road_openings(&self) -> &eo_street::road_openings::RoadOpenings {
        &self.tiles.openings
    }

    /// Every terrain cut (stage A7; none without `--tunnels`), by chain then station.
    pub fn terrain_cuts(&self) -> &[TerrainCut] {
        &self.cuts
    }

    /// Stage A9 (`--tunnels`): the buildings (indices into `rings`, the city's
    /// footprints) whose footprint reaches over a tunnel's approach cut or a
    /// portal's canopy (`terrain_cuts`): a building drawn there stands in the
    /// cut or over the bore's mouth (the Broadway portals, A8), so the host
    /// leaves it out. None without tunnels.
    pub fn buildings_over_cuts(&self, rings: &[impl AsRef<[[f64; 2]]>]) -> Vec<usize> {
        if self.cuts.is_empty() {
            return vec![];
        }
        let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
            let o = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
            let (d1, d2, d3, d4) = (o(a, b, c), o(a, b, d), o(c, d, a), o(c, d, b));
            (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
        };
        let mut out = vec![];
        for (i, footprint) in rings.iter().enumerate() {
            let ring = footprint.as_ref();
            if ring.len() < 3 {
                continue;
            }
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in ring {
                mn = [mn[0].min(p[0]), mn[1].min(p[1])];
                mx = [mx[0].max(p[0]), mx[1].max(p[1])];
            }
            let hit = self.terrain_cuts_in(mn, mx).iter().any(|c| {
                ring.iter().any(|p| c.contains(p[0], p[1]))
                    || c.ring.iter().any(|q| eo_street::point_in_ring(q[0], q[1], ring))
                    || (0..ring.len()).any(|a| (0..c.ring.len()).any(|b| cross(ring[a], ring[(a + 1) % ring.len()], c.ring[b], c.ring[(b + 1) % c.ring.len()])))
            });
            if hit {
                out.push(i);
            }
        }
        out
    }

    /// The terrain cuts whose bounds meet the rectangle `min..max` (plan,
    /// world metres): for a renderer's heightfield tile or a collision tile.
    pub fn terrain_cuts_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<&TerrainCut> {
        let mut ids: Vec<u32> = vec![];
        for gz in index::cell_of(min[1])..=index::cell_of(max[1]) {
            for gx in index::cell_of(min[0])..=index::cell_of(max[0]) {
                if let Some(v) = self.cut_grid.get(&(gx, gz)) {
                    ids.extend(v);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().map(|i| &self.cuts[i as usize]).filter(|c| c.max[0] >= min[0] && c.min[0] <= max[0] && c.max[1] >= min[1] && c.min[1] <= max[1]).collect()
    }

    /// The terrain cut holding (x, z), if any (the lowest floor where cuts overlap).
    pub fn terrain_cut_at(&self, x: f64, z: f64) -> Option<&TerrainCut> {
        let ids = self.cut_grid.get(&(index::cell_of(x), index::cell_of(z)))?;
        ids.iter().map(|&i| &self.cuts[i as usize]).filter(|c| c.contains(x, z)).min_by(|a, b| a.floor.total_cmp(&b.floor))
    }

    /// The options it was built with.
    pub fn options(&self) -> StructureOptions {
        self.options
    }

    /// True when nothing was built: no deck (landmark decks included), no level.
    pub fn is_empty(&self) -> bool {
        !self.chains.iter().any(|c| c.is_built())
    }

    /// The vertical extent of the skyway lights and the roadway they illuminate.
    /// The prop streamer cannot infer these tiles from the at-grade street index.
    pub fn skyway_light_tiles(&self) -> impl Iterator<Item = ((i32, i32), [f32; 2])> + '_ {
        self.tiles.tiles.iter().filter_map(|(&key, tile)| {
            let first = tile.skyway.first()?;
            let mut bounds = [first.road_y as f32, first.top as f32];
            for p in &tile.skyway[1..] {
                bounds[0] = bounds[0].min(p.road_y as f32);
                bounds[1] = bounds[1].max(p.top as f32);
            }
            Some((key, bounds))
        })
    }

    /// Skyway roadway lights owned by the tile containing each shaft.
    pub fn skyway_standards(&self, key: (i32, i32)) -> &[skyway_lights::Standard] {
        self.tiles.tiles.get(&key).map_or(&[], |tile| tile.skyway.as_slice())
    }

    /// Every chain, built or not, by id.
    pub fn chains(&self) -> &[Chain] {
        &self.chains
    }

    pub fn chain(&self, id: u32) -> Option<&Chain> {
        self.chains.get(id as usize)
    }

    /// The chains held (not built), with `Chain::hold` saying why.
    pub fn held(&self) -> impl Iterator<Item = &Chain> {
        self.chains.iter().filter(|c| c.hold().is_some())
    }

    /// Counts of what was built and held, deterministic for the same inputs.
    pub fn census(&self) -> &Census {
        &self.census
    }

    /// Collision triangles for one 128 m tile (the same key as
    /// `eo_street::tile_key`); None when the tile has none. Deck tops of
    /// every built and landmark deck (and the open roads below other ways)
    /// are `street`; barriers, fascia, girders, caps, columns, abutment,
    /// fill and end walls are `barrier`. With `--tunnels` (stage A7) a
    /// bore's roadway is `street`, its walkways, walls, vault, cut walls and
    /// portal headwalls `barrier`.
    pub fn surface_tile(&self, key: (i32, i32)) -> Option<StructureSurface> {
        self.surface_tile_shared(key).map(|s| (*s).clone()).filter(|s| !s.street.is_empty() || !s.barrier.is_empty())
    }

    /// `surface_tile`, shared (built once and kept).
    pub fn surface_tile_shared(&self, key: (i32, i32)) -> Option<Arc<StructureSurface>> {
        self.tiles.surface(&self.chains, key)
    }

    /// Height and slope of the deck at (x, z) for a lane on structure level
    /// `level` (1 = the first level above grade, negative = tunnel), if a
    /// built (or landmark) deck of that level covers the point (within its
    /// half width plus `DECK_QUERY_MARGIN_M`): the nearest such chain,
    /// projected to its station.
    pub fn deck_at(&self, x: f64, z: f64, level: i8) -> Option<DeckPoint> {
        let list = self.decks.get(&(level, index::cell_of(x), index::cell_of(z)))?;
        let mut best: Option<(f64, u32, f64)> = None;
        for &(ci, si) in list {
            let c = &self.chains[ci as usize];
            let i = si as usize;
            let (t, d, _) = index::project([x, z], c.points[i], c.points[i + 1]);
            let s = c.cum[i] + t * (c.cum[i + 1] - c.cum[i]);
            if d > c.half_at(s) + DECK_QUERY_MARGIN_M {
                continue;
            }
            if best.is_none_or(|b| d < b.0 || (d == b.0 && ci < b.1)) {
                best = Some((d, ci, s));
            }
        }
        let (_, ci, s) = best?;
        Some(self.chains[ci as usize].deck_point(s))
    }

    /// The top of built chain `chain`'s deck as drawn over (x, z): its
    /// profile, the street's crown blended in toward a paved end, and a
    /// merged major deck met where it clips this one (`deck_at`'s `y` is the
    /// profile at the centreline; the two differ only within
    /// `sweep::CROWN_BLEND_M` of a paved end, by the crown, and at a gore).
    /// None for a chain with no deck drawn (held, a tunnel), or a point off
    /// its deck (half width plus `DECK_QUERY_MARGIN_M`, past its ends).
    pub fn deck_top(&self, chain: u32, x: f64, z: f64) -> Option<f64> {
        let c = self.chains.get(chain as usize)?;
        // The full-width landmark panels include skewed approach ends past one
        // carriageway's final map node. Traffic and tyres share those faces.
        if let Some(y) = c.landmark.and_then(|deck| self.tiles.landmark_road_height(deck, x, z)) {
            return Some(y);
        }
        let sw = self.tiles.sweeps.get(chain as usize)?.as_ref()?;
        let (s, dist) = c.project(x, z);
        if dist > c.half_at(s) + DECK_QUERY_MARGIN_M || s <= 0.0 || s >= c.length() {
            return None;
        }
        if let Some(y) = mesh::near_deck_height(sw, s, x, z) {
            return Some(y);
        }
        if let Some(y) = self.tiles.join_deck_height(chain, x, z) {
            return Some(y);
        }
        // Where a major deck clips a minor, the major's triangles cover
        // the clipped part. Follow that explicit merge, not a deck nearby.
        for &major in &c.merged_with {
            let other = &self.chains[major as usize];
            if !solve::is_major(other, c) {
                continue;
            }
            let Some(sweep) = self.tiles.sweeps[major as usize].as_ref() else { continue };
            let (s, _) = other.project(x, z);
            if let Some(y) = mesh::near_deck_height(sweep, s, x, z) {
                return Some(y);
            }
        }
        let st = sw.lerp(s);
        let u = (x - st.x) * st.nx + (z - st.z) * st.nz;
        Some(sw.top(&st, u.clamp(-st.half, st.half)))
    }

    /// Every built deck over (x, z), one per level, lowest first.
    pub fn decks_at(&self, x: f64, z: f64) -> Vec<(i8, DeckPoint)> {
        let mut out: Vec<(i8, DeckPoint)> = self.deck_levels.iter().filter_map(|&l| self.deck_at(x, z, l).map(|d| (l, d))).collect();
        out.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.0.cmp(&b.0)));
        out
    }

    /// Level of an OSM way id (`"<id>"` or `"sf-street-<id>"`) for the
    /// simulation: the level of the built chains that carry it, 0 if at
    /// grade, unknown, or only on held chains.
    pub fn way_level(&self, way_id: &str) -> i8 {
        self.levels.get(way_key(way_id)).copied().unwrap_or(0)
    }

    /// Every way a built chain carries, by OSM way id (`"<id>"`, as
    /// `eo_data::sim_source::SimRoad::id` and `eo_data::Road::id` name them),
    /// with its level (`way_level`): the plain-data table the simulation's
    /// lanes take their level from (`eo_sim::native::NativeSource::with_levels`).
    /// Empty for an empty world.
    pub fn way_levels(&self) -> &HashMap<String, i8> {
        &self.levels
    }

    /// The built chain's own height profile, split by source way like the
    /// street plan. These are routing heights and are independent of tile LOD.
    pub fn traffic_surfaces(&self) -> Vec<eo_data::sim_source::TrafficDeckSurface> {
        let mut surfaces = Vec::new();
        let mut lanes = self.landmark_lanes.clone();
        for chain in &self.chains {
            if !chain.is_built() || chain.level == 0 {
                continue;
            }
            for piece in &chain.pieces {
                if piece.s1 <= piece.s0 || self.way_level(&piece.way) != chain.level {
                    continue;
                }
                let start = chain.stations.partition_point(|&s| s <= piece.s0);
                let end = chain.stations.partition_point(|&s| s < piece.s1);
                let sweep = self.tiles.sweeps.get(chain.id as usize).and_then(Option::as_ref);
                let points = std::iter::once(piece.s0)
                    .chain(chain.stations[start..end].iter().copied())
                    .chain(std::iter::once(piece.s1))
                    .map(|s| {
                        let f = chain.frame(s);
                        let y = sweep.filter(|sw| sw.draw != sweep::Draw::Tunnel)
                            .map_or_else(|| chain.deck_point(s).y, |sw| sw.top(&sw.lerp(s), 0.0));
                        [f.x, y, f.z]
                    })
                    .collect();
                surfaces.push(eo_data::sim_source::TrafficDeckSurface {
                    way: piece.way.clone(),
                    points,
                    lanes: lanes.remove(&piece.way).unwrap_or_default(),
                });
            }
        }
        surfaces
    }

    /// Render meshes for one tile at a level of detail (0 near, 1 far, 2
    /// coarse; anything higher is coarse); None when it has nothing to draw.
    /// Landmark decks are not drawn here (the landmark draws them). With
    /// `--tunnels` the built tunnels are (stage A7): roadway as
    /// `DeckSurface`, paint, walkways, walls, vault, cut walls and portals as
    /// `Concrete` (the vault in `tunnel::TILE_TINT`), the crown luminaire as
    /// `Metal`.
    pub fn render_tile(&self, key: (i32, i32), lod: u8) -> Option<StructureMesh> {
        self.render_tile_shared(key, lod).map(|m| (*m).clone())
    }

    /// `render_tile`, shared: built on the first call for (tile, level) and
    /// kept, so repeated calls from any thread are cheap.
    pub fn render_tile_shared(&self, key: (i32, i32), lod: u8) -> Option<Arc<StructureMesh>> {
        self.tiles.mesh(&self.chains, key, lod).filter(|m| !m.is_empty())
    }

    /// Every tile with anything to draw or collide with, sorted.
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        self.tiles.keys()
    }

    /// The columns standing in one tile (by their centre), for keeping
    /// parked cars, trees, lamps and furniture off them (A6); each says
    /// whether it stands in a street's parking lane, furniture zone or
    /// median, and which street.
    pub fn column_footprints(&self, key: (i32, i32)) -> Vec<ColumnFootprint> {
        self.columns.get(&key).cloned().unwrap_or_default()
    }

    /// Every column of every built deck, by tile then chain and bent.
    pub fn all_column_footprints(&self) -> Vec<ColumnFootprint> {
        self.columns.values().flatten().cloned().collect()
    }

    /// Builds every tile at every level (not kept) and counts them: for the
    /// census and the tests, not for the frame.
    pub fn mesh_stats(&self) -> MeshStats {
        let mut st = MeshStats { tiles: self.tiles.keys().len(), ..Default::default() };
        for key in self.tiles.keys() {
            for lod in 0..3u8 {
                let m = self.tiles.build_mesh(&self.chains, key, lod);
                st.triangles[lod as usize] += m.triangles();
                st.vertices[lod as usize] += m.positions.len();
                st.bytes[lod as usize] += m.bytes();
                st.max_tile_triangles[lod as usize] = st.max_tile_triangles[lod as usize].max(m.triangles());
                st.max_tile_vertices[lod as usize] = st.max_tile_vertices[lod as usize].max(m.positions.len());
            }
            let s = self.tiles.build_surface(&self.chains, key);
            st.street_triangles += s.street.len();
            st.barrier_triangles += s.barrier.len();
        }
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_answers_nothing() {
        let w = StructureWorld::empty();
        assert!(w.is_empty());
        assert_eq!(w.options(), StructureOptions::default());
        assert!(w.tile_keys().is_empty());
        for key in [(0, 0), (-1, 3), (12, -8), (i32::MIN, i32::MAX)] {
            assert_eq!(w.surface_tile(key), None);
            for lod in 0..=2 {
                assert_eq!(w.render_tile(key, lod), None);
            }
        }
        for (x, z) in [(0.0, 0.0), (1539.0, -366.0), (-9000.0, 7000.0)] {
            for level in [-2, -1, 0, 1, 2, 5] {
                assert_eq!(w.deck_at(x, z, level), None);
            }
        }
        for id in ["", "8921938", "sf-street-8921938", "not a way"] {
            assert_eq!(w.way_level(id), 0);
        }
    }

    #[test]
    fn shared_between_threads() {
        fn shared<T: Send + Sync + 'static>() {}
        shared::<StructureWorld>();
        shared::<std::sync::Arc<StructureWorld>>();
    }
}
