//! Facade clutter (stage W3, eo-app `--facade-clutter 1|2`, `--no-facade-clutter`):
//! the things San Francisco fronts carry that make a building read as lived
//! in, planned per building from what the planner already knows (archetype,
//! use, storeys, district and age from the district/era prior, the street
//! fronts, the openings, the shop tenancies) and emitted with the rung
//! meshes (`pipeline::build_rung_with`).
//!
//! * Fire escapes on older multi-unit fronts (Tenderloin, Nob Hill,
//!   Chinatown, SoMa, the Mission most): one per stack of windows, a slatted
//!   landing at every floor above the ground floor with a picket railing,
//!   steep stairs between landings through a hatch in the one above, a
//!   raised drop ladder under the lowest, knee brackets, a gooseneck ladder
//!   over the parapet where it reaches; painted (mostly black) steel with
//!   rust by the building's wear. The port's own fire escapes (Edwardian
//!   flats, SoMa warehouses) are drawn this way in their place.
//! * Awnings on shop and food tenancies without a shade: four forms (shed
//!   with valance, quarter dome, box, open retractable), solid colours and
//!   stripes from the common awning fabric colours, faded by the sun, a
//!   few torn.
//! * Sign faces on the fascia boards and on new projecting blade signs (lit
//!   box faces or neon tubes on a dark board) with their brackets. Stage S1:
//!   each carries a generic word of its shop's category (`SIGN_PHRASES`:
//!   CAFE, LIQUOR, DELI, LAUNDRY ...; never a name, a brand or a mark) in one
//!   of four faces, beside an emblem or in a border, painted, back-lit or in
//!   neon, faded by the building's wear: one quad a face whose vertices carry
//!   the word's atlas cell and the text box's (u, v) (`LetterCode`,
//!   `RungMesh::letters`; eo-render draws the words from the system's fonts,
//!   draw.wgsl `sgn_sign`); stacked letters on tall blades; small neon signs
//!   (OPEN, or the trade's word) hung in some bars', liquor stores', cafes',
//!   laundromats' and nail salons' windows. A sign no word fits keeps W3's colour fields and
//!   shapes (discs, rings, diamonds, bands); a lit one carries a glow code in
//!   the vertex normal's spare byte (`RungMesh::glow`, eo-render kit.rs): by
//!   night it emits its own colour where its shop is lit (draw.wgsl `fc_glow`, `fc_lit`).
//! * Roll-down security shutters (hood, guides, curtain down on a share);
//!   level 2: plywood boarding on a few vacant shop fronts.
//! * Downspouts at the party-wall ends of a front, with the conductor head
//!   under the cornice, straps and the bands they pass through.
//! * Electric meters, gas meter sets with their risers, conduits, and the
//!   fire department's standpipe connection at the base.
//! * Roofs: chimneys with clay pots on older buildings, vent stacks and
//!   flue caps, satellite dishes (pointed at the south-south-east sky, as
//!   San Francisco's are), TV aerials on older homes.
//! * Level 2 adds window air conditioners (very rarely: the climate is
//!   mild), dishes on fronts and pipe scaffolding on a building under work.
//!
//! The near rung (0) draws all of it with its thin steel as real bars (no
//! alpha); the mid rung (2) a simplified version (landings as slabs, two
//! rails, stairs as slabs, no slats, pickets, straps or meters); the far
//! level nothing. Nothing is placed through another element, a window
//! frame or a neighbour (every piece is tested against the plan's own
//! projecting parts on that face, from any edge, and against the pieces
//! placed before it), and nothing stands out over the sidewalk past what
//! `Limits` allows. Deterministic per building (`seed|facade-clutter`).
//!
//! Level 0 (the default) adds nothing and the rung meshes are those of
//! before; `cache_tag` is then empty, so the kit tile cache key is today's.

use crate::appearance::building_appearance;
use crate::elevation::{OpeningRecord, Part, Shape};
use crate::frame::Edge;
use crate::mesh::Sink;
use crate::pipeline::{DrawnMaterials, Planned, RungMesh};
use crate::rng::{Rng, hash_seed};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Bump when what is placed or how it is drawn changes (names the kit tile
/// cache folder with the level). 4: keep active food & FiDi storefronts open under high detail.
pub const VERSION: u32 = 4;

static LEVEL: AtomicU8 = AtomicU8::new(0);

/// The level the game draws (eo-app sets it from `--facade-clutter` before
/// the kit city is built); 0 is off.
pub fn set_level(level: u8) {
    LEVEL.store(level.min(2), Ordering::Relaxed);
}

pub fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

/// A tag for the kit tile cache key (empty when off: the key is today's).
pub fn cache_tag() -> String {
    tag_for(level())
}

/// `cache_tag` at a given level.
pub fn tag_for(level: u8) -> String {
    if level == 0 {
        return String::new();
    }
    let l = limits();
    format!("facade-clutter-v{VERSION}-l{level}-{:.3}-{:.3}-{:.3}", l.street_reach_m, l.alley_reach_m, l.frontage_m)
}

// ---------------------------------------------------------------------------
// Limits: what may stand out over the sidewalk and how high above it.
// ---------------------------------------------------------------------------

/// The street standards this module reads (California street standards
/// through `eo_data::standards::CaStandards`), and the code caps the table
/// does not carry (INFERRED constants with their sources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// The most anything may stand out from a street front (m): the
    /// narrowest SF sidewalk class the planner cannot tell apart from the
    /// others (`sidewalk-width-sf-neighborhood-residential`.minimum, 10 ft)
    /// less the edge zone kept clear of vertical elements beside the kerb
    /// (`sidewalk-edge-zone-sf`.verticalElementOffsetFromCurb, 18 in).
    pub street_reach_m: f64,
    /// The same on a service way or alley front (`sidewalk-width-sf-alley`.minimum less the offset).
    pub alley_reach_m: f64,
    /// The frontage zone at the building line (`sidewalk-frontage-zone-sf`.allStreets):
    /// meters, conduits and the standpipe connection stay within it.
    pub frontage_m: f64,
    /// Whether the table was read (false: no street-front clutter, roofs only).
    pub read: bool,
}

/// SF Planning Code Sec. 136.1 (awnings in NC districts: a horizontal
/// projection of at most 4 ft from the face of the building). INFERRED: the
/// code section as recalled, not re-read this session.
pub const AWNING_MAX_PROJECTION_M: f64 = 1.219;
/// SF Planning Code Sec. 607.1 (projecting signs in NC districts: no more than
/// 75 % of the distance from the property line to the curb, and 6 ft 6 in at
/// most). INFERRED as above; the blade signs here stay under 1 m.
pub const BLADE_MAX_PROJECTION_M: f64 = 1.981;
/// Clear height under an awning or sign over the sidewalk: the kit's own
/// (shop_ground.rs `SHOP_AWNING_CLEARANCE_M`, 8 ft, the JS's value).
pub const UNDER_CLEAR_M: f64 = crate::shop_ground::SHOP_AWNING_CLEARANCE_M;

pub fn limits() -> &'static Limits {
    static L: OnceLock<Limits> = OnceLock::new();
    L.get_or_init(|| {
        let none = Limits { street_reach_m: 0.0, alley_reach_m: 0.0, frontage_m: 0.0, read: false };
        let Some(root) = crate::repo_root() else { return none };
        let Ok(ca) = eo_data::CaStandards::load(&eo_data::DataPaths { repo_root: root }) else { return none };
        let get = |id: &str, key: &str| ca.metric_key(id, key).ok();
        let (Some(res), Some(alley), Some(offset), Some(frontage)) = (
            get("sidewalk-width-sf-neighborhood-residential", "minimum"),
            get("sidewalk-width-sf-alley", "minimum"),
            get("sidewalk-edge-zone-sf", "verticalElementOffsetFromCurb"),
            get("sidewalk-frontage-zone-sf", "allStreets"),
        ) else {
            return none;
        };
        Limits { street_reach_m: res - offset, alley_reach_m: alley - offset, frontage_m: frontage, read: true }
    })
}

// ---------------------------------------------------------------------------
// Real-world sizes (named; INFERRED where no standard or survey exists).
// ---------------------------------------------------------------------------

/// Fire escape landing depth: balconies "not less than 3 ft wide" (the 1901
/// New York tenement law San Francisco's 1909 building law followed);
/// SF Tenderloin and Chinatown landings are about 3.5 ft. INFERRED.
const FE_DEPTH_M: f64 = 1.07;
/// Landing rail height (36-42 in on the old escapes). INFERRED.
const FE_RAIL_M: f64 = 0.99;
/// Stairs "not less than 18 in wide" at no more than 60 degrees (same law). INFERRED.
const FE_STAIR_W_M: f64 = 0.5;
const FE_STAIR_RUN_PER_RISE: f64 = 0.577; // tan(30 deg): a 60 degree stair
const FE_TREAD_RISE_M: f64 = 0.215;
/// Deck slats (flat bar on edge) and pickets (square bar), and their pitch. INFERRED.
const FE_SLAT_W_M: f64 = 0.028;
const FE_SLAT_PITCH_M: f64 = 0.056;
const FE_PICKET_M: f64 = 0.016;
const FE_PICKET_PITCH_M: f64 = 0.16;
/// The raised drop ladder's foot stays this far over the sidewalk (out of
/// reach, the reason it is counterweighted up). INFERRED.
const FE_LADDER_FOOT_M: f64 = 2.9;
const FE_LADDER_LEN_M: f64 = 2.3;
const FE_LADDER_W_M: f64 = 0.42;
/// Most a landing may stand off the wall to clear a band or a sill.
const FE_MAX_STANDOFF_M: f64 = 0.24;

/// Awning fabric thickness, frame bar. INFERRED.
const AW_FABRIC_M: f64 = 0.012;
/// Blade sign board thickness and its bracket. INFERRED.
const BLADE_BOARD_M: f64 = 0.07;
/// Fascia sign panel proud of its board.
const FACE_PANEL_M: f64 = 0.012;
/// A rectangular downspout (3 x 4 in), off the wall on its straps. INFERRED.
const DS_W_M: f64 = 0.076;
const DS_D_M: f64 = 0.1;
const DS_STANDOFF_M: f64 = 0.03;

// ---------------------------------------------------------------------------
// Materials and colours.
// ---------------------------------------------------------------------------

/// Painted steel (fire escapes, brackets): the mid grey finish so that a
/// tint (a multiply of at most 1) can reach black, green and oxide red.
const M_STEEL: &str = "painted-steel-mid";
/// Galvanised and light painted metal (conduit, meters, shutters).
const M_GALV: &str = "painted-steel-light";
/// Smooth painted panel (sign faces, plywood, air conditioners): the
/// lightest kit finish, so most colours are a multiply of it.
const M_PANEL: &str = "painted-white";
/// Awning fabric (solid colours that are a multiply of its tan).
const M_CANVAS: &str = "canvas-plain";
/// A dark sign board (neon signs).
const M_BOARD: &str = "signage";
const M_BRICK: &str = "brick-running-bond";
const M_STONE: &str = "cast-stone";
const M_CLAY: &str = "terracotta";

/// sRGB of a kit finish's generator colour (kit-material-specs.json `p.color`
/// or `p.colorA`; white for finishes drawn from their own pattern).
fn finish_srgb(name: &str) -> [u8; 3] {
    static C: OnceLock<HashMap<String, [u8; 3]>> = OnceLock::new();
    let map = C.get_or_init(|| {
        let mut out = HashMap::new();
        let path = eo_data::native_dir().join("eo-assets/data/kit-material-specs.json");
        let Ok(text) = std::fs::read_to_string(path) else { return out };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
        if let Some(m) = v["specs"].as_object() {
            for (k, s) in m {
                let hex = s["p"]["color"].as_str().or(s["p"]["colorA"].as_str());
                if let Some(c) = hex.and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok()) {
                    out.insert(k.clone(), [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                }
            }
        }
        out
    });
    map.get(name).copied().unwrap_or([255, 255, 255])
}

/// The paint (an sRGB multiply, the kit's tint) that draws `target` on `material`.
fn tint(material: &str, target: [u8; 3]) -> u32 {
    let b = finish_srgb(material);
    let c = |k: usize| ((target[k] as f64 / b[k].max(1) as f64) * 255.0).round().clamp(0.0, 255.0) as u32;
    (c(0) << 16) | (c(1) << 8) | c(2)
}

/// The finish to draw a colour on: the first whose generator colour is at
/// least the target in every channel (so the multiply reaches it), else the panel.
fn finish_for(target: [u8; 3], prefer: &'static str) -> &'static str {
    let fits = |m: &str| {
        let b = finish_srgb(m);
        (0..3).all(|k| target[k] <= b[k])
    };
    if fits(prefer) { prefer } else { M_PANEL }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|k| (a[k] as f64 + (b[k] as f64 - a[k] as f64) * t).round() as u8)
}

fn shade(c: [u8; 3], f: f64) -> [u8; 3] {
    c.map(|v| (v as f64 * f).round().clamp(0.0, 255.0) as u8)
}

/// Fire escape paint (sRGB): black most, then dark green, oxide red, grey,
/// with weights (Tenderloin, Chinatown and SoMa fronts). INFERRED.
const FE_PAINTS: [([u8; 3], f64); 4] = [([30, 30, 30], 0.72), ([34, 52, 42], 0.1), ([86, 40, 32], 0.1), ([92, 96, 98], 0.08)];
/// Rust on steel (sRGB), fresh and dark. INFERRED.
const RUST: [[u8; 3]; 2] = [[110, 60, 36], [74, 44, 30]];
/// Solid awning fabric colours (sRGB): forest green, burgundy, navy, black,
/// red, beige, terracotta, blue, teal, charcoal, ochre, hunter green,
/// orange, approximating the common solution-dyed acrylic awning fabrics'
/// published swatches. INFERRED.
const AWNING_SOLIDS: [[u8; 3]; 13] = [
    [38, 66, 52],
    [98, 30, 40],
    [30, 42, 66],
    [36, 36, 38],
    [150, 36, 34],
    [184, 168, 140],
    [150, 74, 50],
    [40, 84, 130],
    [36, 100, 100],
    [76, 78, 80],
    [190, 146, 52],
    [44, 70, 46],
    [190, 92, 40],
];
/// A striped fabric's ground (the white of a stripe, sun-greyed).
const STRIPE_WHITE: [u8; 3] = [222, 218, 206];
/// Where sun-bleached fabric drifts (sRGB).
const BLEACH: [u8; 3] = [200, 194, 182];
/// Sign colours (sRGB): saturated fields and their contrasts. A sign's
/// words (stage S1) are the generic `SIGN_PHRASES` only; one no word fits is
/// fields, bands and shapes.
const SIGN_FIELDS: [[u8; 3]; 12] = [
    [176, 32, 38],
    [222, 214, 196],
    [24, 40, 76],
    [30, 30, 30],
    [214, 168, 40],
    [30, 96, 60],
    [196, 78, 30],
    [120, 40, 110],
    [36, 110, 160],
    [210, 200, 170],
    [150, 20, 70],
    [60, 60, 64],
];
/// Neon tube colours (sRGB): pink, red, blue, green, amber, white.
const NEON: [[u8; 3]; 6] = [[230, 80, 150], [226, 46, 40], [60, 110, 230], [70, 210, 110], [236, 150, 40], [228, 224, 214]];
/// Galvanised and painted service metal (sRGB). INFERRED.
const GALV: [u8; 3] = [150, 154, 152];
const METER_GREY: [u8; 3] = [126, 130, 128];
const BRASS: [u8; 3] = [168, 132, 66];
const FDC_RED: [u8; 3] = [150, 30, 30];
const PLYWOOD: [u8; 3] = [184, 154, 112];
const DISH_WHITE: [u8; 3] = [214, 214, 208];
const AC_BEIGE: [u8; 3] = [206, 200, 186];

/// Glow codes (the vertex normal's w byte): 64..95 a lit box face, 96..127 a
/// neon tube; the low five bits its switch roll (draw.wgsl `fc_glow`). 0 none.
pub const GLOW_BOX: u8 = 64;
pub const GLOW_NEON: u8 = 96;

fn glow_code(neon: bool, roll: u32) -> u8 {
    (if neon { GLOW_NEON } else { GLOW_BOX }) + (roll % 32) as u8
}

// ---------------------------------------------------------------------------
// Lettering (stage S1): generic shop-category words on the sign faces.
// ---------------------------------------------------------------------------

/// Face slots of a lettering cell (eo-props `shop_lettering::FACE_*`, which
/// resolves them to the system's fonts at run time).
pub const FACE_BOLD: u8 = 0;
pub const FACE_CONDENSED: u8 = 1;
pub const FACE_SERIF: u8 = 2;
pub const FACE_SCRIPT: u8 = 3;
const NO_FACE: u8 = u8::MAX;

/// What a sign may say, with the faces it is drawn in (the sign writers'
/// habits: scripts for cafes, bakeries and salons, condensed capitals for
/// liquor and markets). GENERIC WORDS ONLY: shop categories and service
/// descriptions, never a business name, a brand, a trademark, a commercial
/// game, studio or publisher, or anything offensive. Nothing is composed from
/// them (no name prefixes), so every sign reads exactly one of these.
pub const SIGN_PHRASES: &[(&str, [u8; 2])] = &[
    ("CAFE", [FACE_SCRIPT, FACE_BOLD]),
    ("COFFEE", [FACE_SCRIPT, FACE_CONDENSED]),
    ("ESPRESSO", [FACE_SERIF, FACE_SCRIPT]),
    ("COFFEE SHOP", [FACE_SERIF, NO_FACE]),
    ("RESTAURANT", [FACE_SERIF, FACE_SCRIPT]),
    ("DINER", [FACE_BOLD, FACE_SCRIPT]),
    ("NOODLES", [FACE_BOLD, FACE_CONDENSED]),
    ("NOODLE HOUSE", [FACE_SERIF, NO_FACE]),
    ("DIM SUM", [FACE_SERIF, FACE_BOLD]),
    ("TAQUERIA", [FACE_BOLD, FACE_SCRIPT]),
    ("TACOS", [FACE_BOLD, FACE_SCRIPT]),
    ("PIZZA", [FACE_BOLD, FACE_SCRIPT]),
    ("PIZZERIA", [FACE_SCRIPT, FACE_SERIF]),
    ("SUSHI", [FACE_SERIF, FACE_BOLD]),
    ("BURGERS", [FACE_BOLD, FACE_CONDENSED]),
    ("DELI", [FACE_BOLD, FACE_CONDENSED]),
    ("MARKET & DELI", [FACE_CONDENSED, NO_FACE]),
    ("BAR", [FACE_SCRIPT, FACE_SERIF]),
    ("COCKTAILS", [FACE_SCRIPT, FACE_CONDENSED]),
    ("LOUNGE", [FACE_SCRIPT, FACE_SERIF]),
    ("TAVERN", [FACE_SERIF, NO_FACE]),
    ("CLUB", [FACE_CONDENSED, FACE_SCRIPT]),
    ("LIQUOR", [FACE_CONDENSED, FACE_BOLD]),
    ("LIQUORS", [FACE_CONDENSED, NO_FACE]),
    ("LIQUOR & DELI", [FACE_CONDENSED, FACE_BOLD]),
    ("SPIRITS", [FACE_SERIF, NO_FACE]),
    ("WINE", [FACE_SCRIPT, FACE_SERIF]),
    ("WINE BAR", [FACE_SCRIPT, NO_FACE]),
    ("BAKERY", [FACE_SCRIPT, FACE_SERIF]),
    ("BAKERY & CAFE", [FACE_SCRIPT, NO_FACE]),
    ("GROCERY", [FACE_CONDENSED, FACE_BOLD]),
    ("MARKET", [FACE_CONDENSED, FACE_BOLD]),
    ("PRODUCE", [FACE_BOLD, NO_FACE]),
    ("BOOKS", [FACE_SERIF, FACE_CONDENSED]),
    ("USED BOOKS", [FACE_SERIF, NO_FACE]),
    ("RECORDS", [FACE_CONDENSED, FACE_SCRIPT]),
    ("VIDEO", [FACE_CONDENSED, NO_FACE]),
    ("ANTIQUES", [FACE_SERIF, NO_FACE]),
    ("VINTAGE", [FACE_SCRIPT, FACE_SERIF]),
    ("GALLERY", [FACE_SERIF, FACE_BOLD]),
    ("PRINTING", [FACE_BOLD, NO_FACE]),
    ("COPIES", [FACE_CONDENSED, NO_FACE]),
    ("ART SUPPLY", [FACE_BOLD, NO_FACE]),
    ("GIFTS", [FACE_SCRIPT, FACE_SERIF]),
    ("THRIFT", [FACE_CONDENSED, FACE_SCRIPT]),
    ("CLEANERS", [FACE_BOLD, FACE_CONDENSED]),
    ("DRY CLEANERS", [FACE_BOLD, FACE_CONDENSED]),
    ("LAUNDRY", [FACE_BOLD, FACE_CONDENSED]),
    ("COIN LAUNDRY", [FACE_CONDENSED, NO_FACE]),
    ("LAUNDROMAT", [FACE_BOLD, NO_FACE]),
    ("PHARMACY", [FACE_BOLD, FACE_SERIF]),
    ("CLINIC", [FACE_BOLD, NO_FACE]),
    ("DENTAL", [FACE_BOLD, FACE_SERIF]),
    ("OPTICAL", [FACE_SERIF, NO_FACE]),
    ("SALON", [FACE_SCRIPT, FACE_SERIF]),
    ("HAIR SALON", [FACE_SCRIPT, NO_FACE]),
    ("BEAUTY SALON", [FACE_SCRIPT, NO_FACE]),
    ("BARBER", [FACE_SERIF, FACE_CONDENSED]),
    ("BARBER SHOP", [FACE_SERIF, NO_FACE]),
    ("NAILS", [FACE_SCRIPT, FACE_BOLD]),
    ("NAIL SALON", [FACE_SCRIPT, NO_FACE]),
    ("TATTOO", [FACE_SCRIPT, FACE_CONDENSED]),
    ("STUDIO", [FACE_BOLD, FACE_SERIF]),
    ("YOGA", [FACE_SCRIPT, FACE_BOLD]),
    ("FITNESS", [FACE_CONDENSED, NO_FACE]),
    ("GYM", [FACE_CONDENSED, NO_FACE]),
    ("PHOTO", [FACE_BOLD, FACE_CONDENSED]),
    ("TAILOR", [FACE_SERIF, NO_FACE]),
    ("ALTERATIONS", [FACE_SCRIPT, FACE_SERIF]),
    ("SHOES", [FACE_SERIF, FACE_BOLD]),
    ("SHOE REPAIR", [FACE_CONDENSED, NO_FACE]),
    ("REPAIR", [FACE_CONDENSED, NO_FACE]),
    ("FLOWERS", [FACE_SCRIPT, FACE_SERIF]),
    ("FLORIST", [FACE_SCRIPT, NO_FACE]),
    ("FURNITURE", [FACE_SERIF, FACE_CONDENSED]),
    ("RUGS", [FACE_SERIF, NO_FACE]),
    ("LIGHTING", [FACE_BOLD, NO_FACE]),
    ("HARDWARE", [FACE_CONDENSED, FACE_BOLD]),
    ("PAINT", [FACE_BOLD, NO_FACE]),
    ("BIKES", [FACE_BOLD, NO_FACE]),
    ("BIKE SHOP", [FACE_CONDENSED, NO_FACE]),
    ("TAX SERVICE", [FACE_SERIF, NO_FACE]),
    ("INSURANCE", [FACE_SERIF, NO_FACE]),
    ("OPEN", [FACE_SCRIPT, FACE_BOLD]),
];

/// Short words a tall blade sign may carry stacked letter over letter (bold capitals).
pub const STACKED_WORDS: &[&str] = &[
    "BAR", "CAFE", "DELI", "PIZZA", "BOOKS", "NAILS", "SALON", "TATTOO", "LIQUOR", "BAKERY", "MARKET", "WINE", "SUSHI", "TACOS", "DINER", "DENTAL", "YOGA", "PHOTO", "SHOES",
    "CLUB", "VIDEO", "BARBER", "COFFEE", "LOUNGE",
];

/// One layer of the lettering atlas (eo-render `sign_letters` draws it with
/// eo-props `shop_lettering` from the system's fonts): the phrase, the text as
/// drawn (a script in title case), its face slot, whether its letters are
/// stacked, and the text box's aspect (width over height; a stack's length
/// over its width), estimated from the letters so it is the same whatever
/// fonts a machine has (the drawing fits the letters to the box).
#[derive(Clone, Debug, PartialEq)]
pub struct LetterCell {
    pub phrase: &'static str,
    pub text: String,
    pub face: u8,
    pub stacked: bool,
    pub aspect: f64,
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    let mut start = true;
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            out.push(if start { c } else { c.to_ascii_lowercase() });
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// A box's aspect for `text` in face `face` (letter advances in em, the
/// eo-props layout's cap and width shares).
fn estimated_aspect(text: &str, face: u8) -> f64 {
    let em = |c: char| -> f64 {
        let w = match (face, c) {
            (_, ' ') => 0.28,
            (_, '&') => 0.72,
            (FACE_CONDENSED, _) => 0.47,
            (FACE_SCRIPT, c) if c.is_ascii_lowercase() => 0.42,
            (FACE_SCRIPT, _) => 0.62,
            (FACE_SERIF, _) => 0.74,
            _ => 0.71,
        };
        if matches!(c, 'I' | 'J' | 'i' | 'j' | 'l' | 't' | 'f' | 'r') { w * 0.55 } else if matches!(c, 'M' | 'W' | 'm' | 'w') { w * 1.25 } else { w }
    };
    let width: f64 = text.chars().map(em).sum();
    // capitals: box height = cap (0.72 em) / 0.6, text 0.9 of the width; a script: ink about 1.1 em / 0.82.
    let a = if face == FACE_SCRIPT { width / 0.9 / 1.34 } else { width / 0.9 / (0.72 / 0.6) };
    a.max(1.0)
}

/// Every lettering cell, in layer order: each phrase in each of its faces,
/// then the stacked words. At most 256 (the cell is a vertex byte).
pub fn letter_cells() -> &'static [LetterCell] {
    static C: OnceLock<Vec<LetterCell>> = OnceLock::new();
    C.get_or_init(|| {
        let mut v = vec![];
        for (phrase, faces) in SIGN_PHRASES {
            for &face in faces.iter().filter(|f| **f != NO_FACE) {
                let text = if face == FACE_SCRIPT { title_case(phrase) } else { phrase.to_string() };
                let aspect = estimated_aspect(&text, face);
                v.push(LetterCell { phrase, text, face, stacked: false, aspect });
            }
        }
        for w in STACKED_WORDS {
            let n = w.chars().count() as f64;
            v.push(LetterCell { phrase: w, text: w.to_string(), face: FACE_BOLD, stacked: true, aspect: 0.81 * n + 0.15 });
        }
        assert!(v.len() <= 256, "{} lettering cells: the cell is one vertex byte", v.len());
        v
    })
}

/// The cells of a phrase (horizontal ones; or its stacked one).
fn cells_of(phrase: &str, stacked: bool) -> Vec<u8> {
    letter_cells().iter().enumerate().filter(|(_, c)| c.phrase == phrase && c.stacked == stacked).map(|(k, _)| k as u8).collect()
}

/// A tenancy's word (the shop identity's fascia word, shop_ground.rs) to the
/// phrases its signs may carry (all in `SIGN_PHRASES`).
fn family_of_word(w: &str) -> Option<&'static [&'static str]> {
    Some(match w {
        "CAFE" => &["CAFE", "CAFE", "COFFEE", "ESPRESSO", "COFFEE SHOP"],
        "COFFEE" => &["COFFEE", "ESPRESSO", "CAFE"],
        "RESTAURANT" => &["RESTAURANT", "DINER", "NOODLES", "DIM SUM", "TAQUERIA", "PIZZA", "SUSHI", "BURGERS", "NOODLE HOUSE"],
        "DINER" => &["DINER"],
        "DELI" => &["DELI", "DELI", "MARKET & DELI"],
        "PIZZA" | "PIZZERIA" => &["PIZZA", "PIZZERIA"],
        "SUSHI" => &["SUSHI"],
        "NOODLES" => &["NOODLES", "NOODLE HOUSE", "DIM SUM"],
        "TACOS" | "TAQUERIA" => &["TAQUERIA", "TACOS"],
        "BURGERS" => &["BURGERS"],
        "BAR" | "TAVERN" | "PUB" => &["BAR", "BAR", "COCKTAILS", "TAVERN", "LOUNGE"],
        "LOUNGE" => &["LOUNGE", "COCKTAILS"],
        "CLUB" => &["CLUB", "LOUNGE"],
        "LIQUOR" | "LIQUORS" | "SPIRITS" => &["LIQUOR", "LIQUOR", "LIQUORS", "LIQUOR & DELI", "SPIRITS"],
        "WINE" => &["WINE", "WINE BAR"],
        "BAKERY" => &["BAKERY", "BAKERY", "BAKERY & CAFE"],
        "GROCERY" => &["GROCERY", "MARKET", "PRODUCE"],
        "MARKET" => &["MARKET", "MARKET & DELI", "LIQUOR & DELI", "GROCERY"],
        "BOOKS" => &["BOOKS", "BOOKS", "USED BOOKS"],
        "RECORDS" | "MUSIC" => &["RECORDS"],
        "VIDEO" => &["VIDEO"],
        "ANTIQUES" => &["ANTIQUES", "VINTAGE"],
        "GALLERY" => &["GALLERY"],
        "COPIES" | "PRINTING" => &["PRINTING", "COPIES"],
        "SUPPLY" => &["ART SUPPLY", "GIFTS"],
        "CLEANERS" => &["CLEANERS", "DRY CLEANERS"],
        "LAUNDRY" => &["LAUNDRY", "COIN LAUNDRY", "LAUNDROMAT"],
        "PHARMACY" => &["PHARMACY"],
        "CLINIC" => &["CLINIC", "DENTAL", "OPTICAL"],
        "DENTAL" => &["DENTAL"],
        "OPTICAL" => &["OPTICAL"],
        "SALON" => &["SALON", "HAIR SALON", "BEAUTY SALON"],
        "BARBER" => &["BARBER", "BARBER SHOP"],
        "NAILS" => &["NAILS", "NAIL SALON"],
        "TATTOO" => &["TATTOO"],
        "STUDIO" => &["STUDIO", "PHOTO", "YOGA"],
        "TAILOR" => &["TAILOR", "ALTERATIONS"],
        "SHOES" => &["SHOES", "SHOE REPAIR"],
        "FLORIST" | "FLOWERS" => &["FLOWERS", "FLORIST"],
        "FURNITURE" => &["FURNITURE"],
        "RUGS" => &["RUGS"],
        "LIGHTING" => &["LIGHTING"],
        "PAINT" | "HARDWARE" => &["HARDWARE", "HARDWARE", "PAINT"],
        "BIKES" => &["BIKES", "BIKE SHOP"],
        "PHOTO" | "CAMERA" => &["PHOTO"],
        "FITNESS" | "GYM" => &["FITNESS", "GYM"],
        "YOGA" => &["YOGA"],
        _ => return None,
    })
}

/// A shop type (shop_ground.rs `SHOPFRONT_SHOP_TYPES`) to the phrases its
/// signs may carry when the tenancy names no word.
fn family_of_type(t: &str) -> &'static [&'static str] {
    match t {
        "cafe" => &["CAFE", "COFFEE", "DELI", "PIZZA", "TAQUERIA", "NOODLES", "DINER", "RESTAURANT", "SUSHI", "DIM SUM"],
        "bar" => &["BAR", "COCKTAILS", "LIQUOR", "WINE BAR", "LOUNGE", "TAVERN"],
        "bakery" => &["BAKERY", "BAKERY & CAFE"],
        "grocery" => &["MARKET", "GROCERY", "LIQUOR & DELI", "MARKET & DELI", "PRODUCE", "LIQUOR"],
        "books" => &["BOOKS", "THRIFT", "GIFTS", "RECORDS", "VIDEO", "VINTAGE", "ANTIQUES"],
        "cleaners" => &["CLEANERS", "LAUNDRY", "NAILS", "SALON", "BARBER", "PHARMACY", "DENTAL", "TAX SERVICE", "INSURANCE", "COIN LAUNDRY"],
        "studio" => &["STUDIO", "PHOTO", "YOGA", "REPAIR", "SHOE REPAIR", "TAILOR", "FLOWERS", "HARDWARE", "GALLERY", "FURNITURE"],
        _ => &["MARKET", "CAFE", "CLEANERS", "SHOES"],
    }
}

/// The phrases a tenancy's signs may carry: the family of the first known
/// word of its fascia text (the identity's word after '|' first; a name's
/// own words are only looked up, never drawn), else its shop type's.
pub fn sign_family(text: Option<&str>, shop_type: &str) -> &'static [&'static str] {
    text.and_then(|t| {
        let up = t.to_uppercase();
        up.split('|').rev().find_map(|part| part.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty()).find_map(family_of_word))
    })
    .unwrap_or_else(|| family_of_type(shop_type))
}

/// Letter inks (sRGB): cream, black, gold, red, pink, blue (draw.wgsl
/// `SL_INKS`); a neon tube takes its colour from the same list.
pub const SIGN_INKS: [[u8; 3]; 6] = [[240, 232, 214], [24, 24, 26], [222, 172, 52], [196, 34, 38], [236, 92, 160], [44, 92, 196]];
const INK_CREAM: u8 = 0;
const INK_BLACK: u8 = 1;
const INK_GOLD: u8 = 2;
const INK_RED: u8 = 3;
const INK_PINK: u8 = 4;
const INK_BLUE: u8 = 5;
/// Where sun-bleached sign paint drifts (sRGB), and the fades a field may
/// carry. INFERRED: the reference sheet has no sign fading data; a red field
/// in the sun goes pink-grey within a few years.
const SIGN_BLEACH: [u8; 3] = [214, 206, 192];
pub const SIGN_FADES: [f64; 3] = [0.0, 0.22, 0.42];
/// A neon sign's dark board (sRGB).
const NEON_BOARD: [u8; 3] = [28, 30, 34];
/// The fields a lettered face is painted: SIGN_FIELDS at each fade, then the neon board.
pub const SIGN_FIELD_COUNT: usize = SIGN_FIELDS.len() * SIGN_FADES.len() + 1;
const FIELD_NEON_BOARD: u8 = (SIGN_FIELD_COUNT - 1) as u8;

/// A lettered field's colour (sRGB).
pub fn sign_field_colour(i: usize) -> [u8; 3] {
    if i >= SIGN_FIELD_COUNT - 1 {
        return NEON_BOARD;
    }
    mix(SIGN_FIELDS[i % SIGN_FIELDS.len()], SIGN_BLEACH, SIGN_FADES[i / SIGN_FIELDS.len()])
}

/// The paint (sRGB multiply bytes) that draws field `i` on the panel finish:
/// draw.wgsl `SL_FIELDS` holds these over 255 (a test keeps them equal).
pub fn sign_field_tint(i: usize) -> [u8; 3] {
    let c = sign_field_colour(i);
    let t = tint(finish_for(c, M_PANEL), c);
    [(t >> 16) as u8, (t >> 8) as u8, t as u8]
}

/// Lettering modes: a painted face; a lit sign whose letters glow over a
/// dimmer face (channel letters, a back-lit sign with an opaque face); a face
/// lit through evenly; neon tubes on a dark board.
pub const LETTER_PAINTED: u8 = 0;
pub const LETTER_LIT_LETTERS: u8 = 1;
pub const LETTER_LIT_FACE: u8 = 2;
pub const LETTER_NEON: u8 = 3;
/// Emblems beside the text: none, a disc, a ring or a diamond on the left or
/// right, or a border round it (a neon tube on a neon sign).
pub const EMBLEM_NONE: u8 = 0;
pub const EMBLEM_BORDER: u8 = 7;
/// u and v of the text box a lettered vertex byte spans (byte 0 = -7.5, 255 = 8.5).
pub const LETTER_UV_MIN: f64 = -7.5;
pub const LETTER_UV_SPAN: f64 = 16.0;

/// What a lettered sign quad carries (draw.wgsl `sl_sign`). `query` says
/// which way the shop's front faces, for the shop light that switches it:
/// 0 the quad's own normal (a fascia), 1 the viewer's right of it, 2 the left
/// (a blade sign's sides).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LetterCode {
    pub cell: u8,
    pub mode: u8,
    pub query: u8,
    pub ink: u8,
    pub field: u8,
    pub emblem: u8,
}

impl LetterCode {
    fn packed(&self) -> u32 {
        (((self.emblem as u32 * 4 + self.mode as u32) * 3 + self.query as u32) * 6 + self.ink as u32) * SIGN_FIELD_COUNT as u32 + self.field as u32
    }

    /// The five bytes of a lettered vertex at text-box (u, v): rgb = (u, v,
    /// cell), the normal's y and w = the rest (y - 127 + 127 * w-code, w
    /// negative: a non-glass kit vertex's w is never negative otherwise).
    pub fn bytes(&self, u: f64, v: f64) -> [u8; 5] {
        let q = |t: f64| (((t - LETTER_UV_MIN) / LETTER_UV_SPAN) * 255.0).round().clamp(0.0, 255.0) as u8;
        let code = self.packed();
        let (yi, wi) = ((code / 127) as i32, (code % 127) as i32);
        [q(u), q(v), self.cell, (yi - 127) as i8 as u8, (-wi - 1) as i8 as u8]
    }

    /// `bytes` read back (as draw.wgsl `sl_decode` reads them): the code and (u, v).
    pub fn decode(b: [u8; 5]) -> Option<(Self, f64, f64)> {
        let (y, w) = (b[3] as i8 as i32, b[4] as i8 as i32);
        if w >= 0 || w == -128 || y == -128 {
            return None;
        }
        let mut code = ((y + 127) * 127 + (-w - 1)) as u32;
        let field = (code % SIGN_FIELD_COUNT as u32) as u8;
        code /= SIGN_FIELD_COUNT as u32;
        let ink = (code % 6) as u8;
        code /= 6;
        let query = (code % 3) as u8;
        code /= 3;
        let mode = (code % 4) as u8;
        let emblem = (code / 4) as u8;
        let t = |x: u8| x as f64 / 255.0 * LETTER_UV_SPAN + LETTER_UV_MIN;
        Some((Self { cell: b[2], mode, query, ink, field, emblem }, t(b[0]), t(b[1])))
    }
}

/// A lettered sign's design (planned with the sign; drawn per rung).
#[derive(Clone, Debug)]
struct Lettering {
    code: LetterCode,
    /// The cell's text box aspect, and the box's height (m; across a stack).
    aspect: f64,
    height: f64,
    /// Fascia: the text box's centre along the face (s) and up (y).
    centre: [f64; 2],
    /// Blade: 0 across the board, 1 turned to read upward, 2 stacked.
    layout: u8,
    /// The group's glow code (`RungMesh::glow`: a lit sign is a lit face).
    glow: u8,
}

fn luminance(c: [u8; 3]) -> f64 {
    let l = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * l(c[0]) + 0.7152 * l(c[1]) + 0.0722 * l(c[2])
}

/// An ink that reads on `field` (sRGB): a dark field takes cream, gold or
/// red, a light one black, red or blue, a mid-tone one cream or black.
fn ink_for(field: [u8; 3], r: f64) -> u8 {
    let lf = luminance(field);
    let reddish = field[0] as i32 > field[1] as i32 + 60 && field[0] as i32 > field[2] as i32 + 40;
    let pick = if lf < 0.06 {
        weighted(&[(INK_CREAM, 0.55), (INK_GOLD, 0.3), (if reddish { INK_CREAM } else { INK_RED }, 0.15)], r)
    } else if lf > 0.45 {
        weighted(&[(INK_BLACK, 0.55), (if reddish { INK_BLACK } else { INK_RED }, 0.27), (INK_BLUE, 0.18)], r)
    } else {
        weighted(&[(INK_CREAM, 0.78), (if lf > 0.2 { INK_BLACK } else { INK_GOLD }, 0.22)], r)
    };
    pick
}

/// The ink nearest a neon tube colour (pink, red, blue, gold, cream).
fn neon_ink(c: [u8; 3]) -> u8 {
    [INK_PINK, INK_RED, INK_BLUE, INK_GOLD, INK_CREAM]
        .into_iter()
        .min_by_key(|&k| (0..3).map(|j| (SIGN_INKS[k as usize][j] as i32 - c[j] as i32).pow(2)).sum::<i32>())
        .unwrap_or(INK_PINK)
}

/// A field's palette index (`sign_field_colour`): W3's colour at a fade.
fn field_index(field: [u8; 3], fade: usize) -> u8 {
    let k = SIGN_FIELDS.iter().position(|f| *f == field).unwrap_or(0);
    (fade.min(SIGN_FADES.len() - 1) * SIGN_FIELDS.len() + k) as u8
}

/// The fade a sign carries by its building's paint wear (0 new .. 1 worn).
fn sign_fade(paint: f64, r: f64) -> usize {
    let worn = 0.12 + 0.3 * paint.clamp(0.0, 1.0);
    if r < worn * 0.45 {
        2
    } else if r < worn * 1.4 {
        1
    } else {
        0
    }
}

/// Phrases a fascia may carry in neon (bars, liquor stores, late food).
fn neon_phrase(p: &str) -> bool {
    matches!(p, "BAR" | "COCKTAILS" | "LOUNGE" | "LIQUOR" | "LIQUORS" | "WINE" | "WINE BAR" | "TATTOO" | "PIZZA" | "CLUB" | "TAVERN" | "SUSHI" | "DINER" | "OPEN")
}

/// A fascia sign's lettering: the panel (s0..s1, y0..y1) carries one of the
/// tenancy's phrases centred on the part of it nothing in front hides (a
/// canopy over the shop often covers the band's foot), beside an emblem or
/// in a border, its letters most of that height; on a long panel a phrase
/// that fills it rather than a short word lost in it. None when no phrase
/// fits (it keeps its emblems). `right_s`: +1 when the reader's right runs
/// along the face's s, else -1.
#[allow(clippy::too_many_arguments)]
fn letter_fascia(f: &Face, sf: &SignFace, family: &[&'static str], paint: f64, lit: bool, key: u32, right_s: f64, r: &mut Rng) -> Option<Lettering> {
    let pw = sf.s1 - sf.s0;
    let first = pick(family, r.next());
    let face_roll = r.next();
    let emblem_roll = r.next();
    let emblem_kind = r.next();
    let ink_roll = r.next();
    let fade = sign_fade(paint, r.next());
    let mode_roll = r.next();
    let neon_p = if crate::typology::detail_enabled() { 0.38 } else { 0.16 };
    let neon = lit && neon_phrase(first) && mode_roll < neon_p;
    // the band in sight: anything standing out in front of the panel over most of it cuts it.
    let front = sf.w + FACE_PANEL_M;
    let (mut vy0, mut vy1) = (sf.y0, sf.y1);
    for o in f.obstacles.iter().chain(&f.placed) {
        let over = o.s1.min(sf.s1) - o.s0.max(sf.s0);
        if o.w1 <= front + 0.005 || over <= 0.5 * pw {
            continue;
        }
        if o.y1 > vy0 && o.y0 < vy1 {
            if (o.y0 + o.y1) / 2.0 < (sf.y0 + sf.y1) / 2.0 {
                vy0 = vy0.max(o.y1);
            } else {
                vy1 = vy1.min(o.y0);
            }
        } else if o.y1 <= sf.y0 + 1e-6 && o.y1 > sf.y0 - 0.6 {
            // a canopy just under the board, seen from the walk across the street (eye 1.6 m,
            // about 10 m off, the board some 2 m above the eye: 0.2 m up a metre out), hides its foot.
            vy0 = vy0.max(o.y1 + 0.2 * (o.w1 - front));
        }
    }
    let vh = vy1 - vy0;
    if vh < 0.18 {
        return None;
    }
    let side_wanted = !neon && emblem_roll < 0.42;
    let border = (neon && emblem_roll < 0.7) || (!neon && emblem_roll > 0.8);
    // every cell of the family: the drawn phrase's own faces first (its face roll's first), then the others.
    let mut order: Vec<u8> = vec![];
    let own = cells_of(first, false);
    if !own.is_empty() {
        let k = ((face_roll * own.len() as f64) as usize).min(own.len() - 1);
        order.push(own[k]);
        order.extend(own.iter().copied().filter(|c| *c != own[k]));
    }
    for p in family {
        for c in cells_of(p, false) {
            if !order.contains(&c) {
                order.push(c);
            }
        }
    }
    // (cell, box height, side emblem, its share of the panel)
    let fit = |cell: u8| -> Option<(u8, f64, bool, f64)> {
        let aspect = letter_cells()[cell as usize].aspect;
        let room = pw - 0.3 * vh;
        let (h_frac, h_cap) = if crate::typology::detail_enabled() {
            (if border { 0.70 } else { 0.86 }, 0.80)
        } else {
            (if border { 0.66 } else { 0.8 }, 0.7)
        };
        let mut h = (vh * h_frac).min(h_cap);
        let side = side_wanted && room > aspect * h + 1.3 * h;
        let reserve = if side { 1.25 * h } else if border { 0.55 * h } else { 0.0 };
        if aspect * h > room - reserve {
            h = (room - reserve) / aspect;
        }
        let shift = if side { 0.62 * h } else { 0.0 };
        // a box under 14 cm tall reads as a smudge; the panel's far end must
        // lie within the u (and v) the vertex bytes address (8 boxes from the centre).
        let u_ok = pw / 2.0 + shift <= 7.9 * aspect * h;
        let yc = (vy0 + vy1) / 2.0;
        let v_ok = (yc - sf.y0).max(sf.y1 - yc) <= 7.9 * h;
        (h >= 0.14 && u_ok && v_ok).then_some((cell, h, side, (aspect * h + reserve) / pw))
    };
    let fits: Vec<(u8, f64, bool, f64)> = order.iter().filter_map(|c| fit(*c)).collect();
    let best = fits.iter().map(|f| f.3).fold(0.0, f64::max);
    // the drawn phrase when it fills its panel well enough, else whichever fills it best.
    let (cell, h, side, _) = *fits.iter().find(|f| f.3 >= 0.42 || f.3 >= 0.8 * best).or_else(|| fits.iter().max_by(|a, b| a.3.total_cmp(&b.3)))?;
    let aspect = letter_cells()[cell as usize].aspect;
    let left = emblem_kind < 0.5;
    // the text moves away from its emblem (to the reader's right of a left one).
    let shift = if side { 0.62 * h * right_s * if left { 1.0 } else { -1.0 } } else { 0.0 };
    let emblem = if side { 1 + 2 * ((emblem_kind * 6.0) as u8 % 3) + if left { 0 } else { 1 } } else if border { EMBLEM_BORDER } else { EMBLEM_NONE };
    let centre = [(sf.s0 + sf.s1) / 2.0 + shift, (vy0 + vy1) / 2.0];
    let (mode, field, ink) = if neon {
        (LETTER_NEON, FIELD_NEON_BOARD, neon_ink(pick(&NEON, ink_roll)))
    } else {
        let mut ink = ink_for(sf.field, ink_roll);
        let lf = luminance(sf.field);
        let dark = lf < 0.08;
        // Under high-detail street mode, saturated/dark sign boards use high-contrast cream/gold backlit
        // channel letters (`LETTER_LIT_LETTERS`) rather than solid over-bright glowing bars (`LETTER_LIT_FACE`).
        if crate::typology::detail_enabled() && lit && lf < 0.30 && luminance(SIGN_INKS[ink as usize]) <= 0.12 {
            ink = if ink_roll < 0.72 { INK_CREAM } else { INK_GOLD };
        }
        // letters lit over a dimmer face only where they are light (a dark ink is printed on a lit face).
        let bright = luminance(SIGN_INKS[ink as usize]) > 0.12;
        let mode = if !lit {
            LETTER_PAINTED
        } else if bright && (crate::typology::detail_enabled() && mode_roll < 0.88 || (dark && mode_roll < 0.72) || (!dark && mode_roll < 0.3)) {
            LETTER_LIT_LETTERS
        } else {
            LETTER_LIT_FACE
        };
        (mode, field_index(sf.field, fade), ink)
    };
    let glow = if mode == LETTER_PAINTED { 0 } else { glow_code(mode == LETTER_NEON, key) };
    Some(Lettering { code: LetterCode { cell, mode, query: 0, ink, field, emblem }, aspect, height: h, centre, layout: 0, glow })
}

/// A blade sign's lettering, the same on both faces: a stacked word on a tall
/// board, a word turned to read upward on a narrow one, else across it.
fn letter_blade(bl: &Blade, family: &[&'static str], paint: f64, key: u32, r: &mut Rng) -> Option<Lettering> {
    let (pw, ph) = (bl.w1 - bl.w0, bl.y1 - bl.y0);
    let phrase = pick(family, r.next());
    let face_roll = r.next();
    let ink_roll = r.next();
    let fade = sign_fade(paint, r.next());
    let mode_roll = r.next();
    let border_roll = r.next();
    let lit = bl.glow != 0;
    let stack = cells_of(phrase, true);
    let flat = cells_of(phrase, false);
    // (layout, cell, text box height across the board)
    let mut choice: Option<(u8, u8, f64)> = None;
    if ph > 1.45 * pw && !stack.is_empty() {
        let c = stack[0];
        let a = letter_cells()[c as usize].aspect;
        let h = (0.66 * pw).min(0.86 * ph / a);
        if h >= 0.16 {
            choice = Some((2, c, h));
        }
    }
    if choice.is_none() && !flat.is_empty() {
        let c = flat[((face_roll * flat.len() as f64) as usize).min(flat.len() - 1)];
        let a = letter_cells()[c as usize].aspect;
        let across = (0.3 * ph).min(0.84 * pw / a);
        let upward = (0.5 * pw).min(0.84 * ph / a);
        if across >= 0.1 && across >= upward * 0.8 {
            choice = Some((0, c, across));
        } else if upward >= 0.1 {
            choice = Some((1, c, upward));
        }
    }
    let (layout, cell, h) = choice?;
    let aspect = letter_cells()[cell as usize].aspect;
    let (mode, field, ink) = if bl.neon {
        (LETTER_NEON, FIELD_NEON_BOARD, neon_ink(bl.tubes))
    } else {
        let dark = luminance(bl.field) < 0.08;
        let mode = if !lit {
            LETTER_PAINTED
        } else if (dark && mode_roll < 0.6) || (!dark && mode_roll < 0.25) {
            LETTER_LIT_LETTERS
        } else {
            LETTER_LIT_FACE
        };
        (mode, field_index(bl.field, fade), ink_for(bl.field, ink_roll))
    };
    // a neon board's tube runs round its edge; a box board sometimes carries a painted border.
    let emblem = if (bl.neon && border_roll < 0.8) || (!bl.neon && border_roll < 0.25 && layout != 2) { EMBLEM_BORDER } else { EMBLEM_NONE };
    let glow = if mode == LETTER_PAINTED { 0 } else { glow_code(mode == LETTER_NEON, key) };
    Some(Lettering { code: LetterCode { cell, mode, query: 1, ink, field, emblem }, aspect, height: h, centre: [0.0, 0.0], layout, glow })
}

// ---------------------------------------------------------------------------
// Geometry.
// ---------------------------------------------------------------------------

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn norm(a: V3) -> V3 {
    let l = len(a).max(1e-12);
    scale(a, 1.0 / l)
}
fn neg(a: V3) -> V3 {
    scale(a, -1.0)
}

fn quad(sink: &mut Sink, a: V3, b: V3, c: V3, d: V3, n: V3) {
    let base = sink.positions.len() as u32;
    let nf = [n[0] as f32, n[1] as f32, n[2] as f32];
    for p in [a, b, c, d] {
        sink.positions.push(p);
        sink.normals.push(nf);
    }
    sink.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// A convex polygon (fan from its first point) with one normal.
fn fan(sink: &mut Sink, pts: &[V3], n: V3) {
    if pts.len() < 3 {
        return;
    }
    let base = sink.positions.len() as u32;
    let nf = [n[0] as f32, n[1] as f32, n[2] as f32];
    for p in pts {
        sink.positions.push(*p);
        sink.normals.push(nf);
    }
    for i in 1..pts.len() as u32 - 1 {
        sink.indices.extend_from_slice(&[base, base + i, base + i + 1]);
    }
}

/// An edge basis: s along the wall, w out of it (world x, z); y is world height.
#[derive(Clone, Copy, Debug)]
struct Eb {
    o: [f64; 2],
    u: [f64; 2],
    n: [f64; 2],
}

impl Eb {
    fn of(e: &Edge) -> Self {
        Self { o: e.basis.origin, u: e.basis.u, n: e.basis.n }
    }
    fn p(&self, s: f64, y: f64, w: f64) -> V3 {
        [self.o[0] + self.u[0] * s + self.n[0] * w, y, self.o[1] + self.u[1] * s + self.n[1] * w]
    }
    fn du(&self) -> V3 {
        [self.u[0], 0.0, self.u[1]]
    }
    fn dn(&self) -> V3 {
        [self.n[0], 0.0, self.n[1]]
    }
    /// (s, w) of a world (x, z).
    fn local(&self, x: f64, z: f64) -> (f64, f64) {
        let (dx, dz) = (x - self.o[0], z - self.o[1]);
        (dx * self.u[0] + dz * self.u[1], dx * self.n[0] + dz * self.n[1])
    }
}

const F_OUT: u8 = 1;
const F_IN: u8 = 2;
const F_S1: u8 = 4;
const F_S0: u8 = 8;
const F_TOP: u8 = 16;
const F_BOT: u8 = 32;
const F_ALL: u8 = 63;
/// No face against the wall.
const F_NO_BACK: u8 = F_ALL & !F_IN;
/// The four long faces of a bar along s.
const F_ALONG_S: u8 = F_OUT | F_IN | F_TOP | F_BOT;
/// The four long faces of a bar along y.
const F_ALONG_Y: u8 = F_OUT | F_IN | F_S0 | F_S1;
/// The four long faces of a bar along w.
const F_ALONG_W: u8 = F_S0 | F_S1 | F_TOP | F_BOT;

/// A box in an edge basis (the faces in `faces`).
#[allow(clippy::too_many_arguments)]
fn ebox(sink: &mut Sink, e: &Eb, s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64, faces: u8) {
    let (u, n) = (e.du(), e.dn());
    let p = |s, y, w| e.p(s, y, w);
    if faces & F_OUT != 0 {
        quad(sink, p(s0, y0, w1), p(s1, y0, w1), p(s1, y1, w1), p(s0, y1, w1), n);
    }
    if faces & F_IN != 0 {
        quad(sink, p(s1, y0, w0), p(s0, y0, w0), p(s0, y1, w0), p(s1, y1, w0), neg(n));
    }
    if faces & F_S1 != 0 {
        quad(sink, p(s1, y0, w1), p(s1, y0, w0), p(s1, y1, w0), p(s1, y1, w1), u);
    }
    if faces & F_S0 != 0 {
        quad(sink, p(s0, y0, w0), p(s0, y0, w1), p(s0, y1, w1), p(s0, y1, w0), neg(u));
    }
    if faces & F_TOP != 0 {
        quad(sink, p(s0, y1, w1), p(s1, y1, w1), p(s1, y1, w0), p(s0, y1, w0), [0.0, 1.0, 0.0]);
    }
    if faces & F_BOT != 0 {
        quad(sink, p(s0, y0, w0), p(s1, y0, w0), p(s1, y0, w1), p(s0, y0, w1), [0.0, -1.0, 0.0]);
    }
}

/// A bar of rectangular section (half sizes `hx` across, `hy` along `up`
/// projected) from `a` to `b`: its four long faces (and its ends with `caps`).
fn bar(sink: &mut Sink, a: V3, b: V3, hx: f64, hy: f64, up: V3, caps: bool) {
    let d = norm(sub(b, a));
    let mut x = cross(d, up);
    if len(x) < 1e-6 {
        x = cross(d, [1.0, 0.0, 0.0]);
        if len(x) < 1e-6 {
            x = cross(d, [0.0, 0.0, 1.0]);
        }
    }
    let x = norm(x);
    let y = norm(cross(x, d));
    let c = |p: V3, sx: f64, sy: f64| add(p, add(scale(x, sx * hx), scale(y, sy * hy)));
    let (a00, a10, a11, a01) = (c(a, -1.0, -1.0), c(a, 1.0, -1.0), c(a, 1.0, 1.0), c(a, -1.0, 1.0));
    let (b00, b10, b11, b01) = (c(b, -1.0, -1.0), c(b, 1.0, -1.0), c(b, 1.0, 1.0), c(b, -1.0, 1.0));
    quad(sink, a10, b10, b11, a11, x);
    quad(sink, b00, a00, a01, b01, neg(x));
    quad(sink, a11, b11, b01, a01, y);
    quad(sink, a00, b00, b10, a10, neg(y));
    if caps {
        quad(sink, a00, a10, a11, a01, neg(d));
        quad(sink, b10, b00, b01, b11, d);
    }
}

/// A square bar from `a` to `b` (half size `h`).
fn rod(sink: &mut Sink, a: V3, b: V3, h: f64) {
    bar(sink, a, b, h, h, [0.0, 1.0, 0.0], false);
}

/// An n-sided prism of radius `r` along `a` -> `b` (round pipes, pots, discs).
fn cyl(sink: &mut Sink, a: V3, b: V3, r: f64, sides: usize, caps: bool) {
    let d = norm(sub(b, a));
    let mut x = cross(d, [0.0, 1.0, 0.0]);
    if len(x) < 1e-6 {
        x = [1.0, 0.0, 0.0];
    }
    let x = norm(x);
    let y = cross(x, d);
    let ring = |c: V3| -> Vec<V3> {
        (0..sides)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / sides as f64;
                add(c, add(scale(x, r * t.cos()), scale(y, r * t.sin())))
            })
            .collect()
    };
    let (ra, rb) = (ring(a), ring(b));
    for i in 0..sides {
        let j = (i + 1) % sides;
        let t = std::f64::consts::TAU * (i as f64 + 0.5) / sides as f64;
        let n = add(scale(x, t.cos()), scale(y, t.sin()));
        quad(sink, ra[i], ra[j], rb[j], rb[i], n);
    }
    if caps {
        let rev: Vec<V3> = ra.iter().rev().copied().collect();
        fan(sink, &rev, neg(d));
        fan(sink, &rb, d);
    }
}

/// A flat convex plate in the plane (o, ax, ay), from `t0` to `t1` along its
/// normal `n` (= ax x ay): its two faces and its rim.
fn plate(sink: &mut Sink, o: V3, ax: V3, ay: V3, pts: &[[f64; 2]], t0: f64, t1: f64, both: bool) {
    let n = norm(cross(ax, ay));
    let at = |p: &[f64; 2], t: f64| add(o, add(add(scale(ax, p[0]), scale(ay, p[1])), scale(n, t)));
    let front: Vec<V3> = pts.iter().map(|p| at(p, t1)).collect();
    fan(sink, &front, n);
    if both {
        let back: Vec<V3> = pts.iter().rev().map(|p| at(p, t0)).collect();
        fan(sink, &back, neg(n));
    }
    for i in 0..pts.len() {
        let j = (i + 1) % pts.len();
        let e = sub(at(&pts[j], t1), at(&pts[i], t1));
        let side = norm(cross(e, n));
        quad(sink, at(&pts[i], t0), at(&pts[j], t0), at(&pts[j], t1), at(&pts[i], t1), side);
    }
}

/// A closed (w, y) profile swept along s from `s0` to `s1` (and its end caps).
fn sweep(sink: &mut Sink, e: &Eb, prof: &[[f64; 2]], s0: f64, s1: f64, caps: bool) {
    let n = prof.len();
    if n < 3 {
        return;
    }
    let mut area = 0.0;
    for i in 0..n {
        let ([w0, y0], [w1, y1]) = (prof[i], prof[(i + 1) % n]);
        area += w0 * y1 - w1 * y0;
    }
    let ccw = area > 0.0;
    let (u, nn) = (e.du(), e.dn());
    for i in 0..n {
        let ([wa, ya], [wb, yb]) = (prof[i], prof[(i + 1) % n]);
        let (dw, dy) = (wb - wa, yb - ya);
        let l = dw.hypot(dy);
        if l < 1e-6 {
            continue;
        }
        let (mut pw, mut py) = (dy / l, -dw / l);
        if !ccw {
            pw = -pw;
            py = -py;
        }
        let normal = [nn[0] * pw, py, nn[2] * pw];
        quad(sink, e.p(s0, ya, wa), e.p(s1, ya, wa), e.p(s1, yb, wb), e.p(s0, yb, wb), normal);
    }
    if caps {
        let mut tri: Vec<u32> = vec![];
        let mut ear = earcut::Earcut::<f64>::new();
        ear.earcut(prof.iter().copied(), &[] as &[u32], &mut tri);
        for (s, nrm, flip) in [(s1, u, false), (s0, neg(u), true)] {
            let base = sink.positions.len() as u32;
            let nf = [nrm[0] as f32, nrm[1] as f32, nrm[2] as f32];
            for [w, y] in prof {
                sink.positions.push(e.p(s, *y, *w));
                sink.normals.push(nf);
            }
            for t in tri.chunks_exact(3) {
                if flip {
                    sink.indices.extend_from_slice(&[base + t[0], base + t[2], base + t[1]]);
                } else {
                    sink.indices.extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The mesh the clutter adds: groups by (finish, paint, glow).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Mesher {
    groups: Vec<(String, u32, u8, Sink)>,
    /// (stage S1) per group, its vertices' lettering bytes (`LetterCode::bytes`); None for a plain group.
    codes: Vec<Option<Vec<[u8; 5]>>>,
}

impl Mesher {
    fn at(&mut self, material: &str, paint: u32, glow: u8) -> &mut Sink {
        let k = match (0..self.groups.len()).position(|k| self.codes[k].is_none() && self.groups[k].0 == material && self.groups[k].1 == paint && self.groups[k].2 == glow) {
            Some(k) => k,
            None => {
                self.groups.push((material.to_string(), paint, glow, Sink::default()));
                self.codes.push(None);
                self.groups.len() - 1
            }
        };
        &mut self.groups[k].3
    }
    /// The lettered group of a glow code (on the panel finish; the vertex
    /// bytes carry its colours): its sink and its vertices' bytes.
    fn lettered(&mut self, glow: u8) -> (&mut Sink, &mut Vec<[u8; 5]>) {
        let k = match (0..self.groups.len()).position(|k| self.codes[k].is_some() && self.groups[k].2 == glow) {
            Some(k) => k,
            None => {
                self.groups.push((M_PANEL.to_string(), 0xffffff, glow, Sink::default()));
                self.codes.push(Some(vec![]));
                self.groups.len() - 1
            }
        };
        let codes = self.codes[k].as_mut().expect("a lettered group");
        (&mut self.groups[k].3, codes)
    }
    /// A colour on its finish (`prefer` if the multiply reaches it).
    fn col(&mut self, prefer: &'static str, c: [u8; 3], glow: u8) -> &mut Sink {
        let m = finish_for(c, prefer);
        let t = tint(m, c);
        self.at(m, t, glow)
    }
}

// ---------------------------------------------------------------------------
// Obstacles: what already stands out of a face, in that face's basis.
// ---------------------------------------------------------------------------

/// An axis box in an edge basis: s along, y up (world), w out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bx {
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
    pub w0: f64,
    pub w1: f64,
}

impl Bx {
    pub fn new(s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64) -> Self {
        Self { s0: s0.min(s1), s1: s0.max(s1), y0: y0.min(y1), y1: y0.max(y1), w0: w0.min(w1), w1: w0.max(w1) }
    }
    /// Overlap by more than `eps` on every axis.
    pub fn hits(&self, o: &Bx, eps: f64) -> bool {
        self.s0 < o.s1 - eps && o.s0 < self.s1 - eps && self.y0 < o.y1 - eps && o.y0 < self.y1 - eps && self.w0 < o.w1 - eps && o.w0 < self.w1 - eps
    }
}

/// The world-space corners of a part's bounds (box, profile and plate parts
/// in their basis; prisms by their polygon).
fn part_corners(p: &Part) -> Vec<V3> {
    match &p.shape {
        Shape::Prism(pts) => pts.iter().flat_map(|q| [[q.x, p.y0, q.z], [q.x, p.y1, q.z]]).collect(),
        _ => {
            let e = Eb { o: p.basis.origin, u: p.basis.u, n: p.basis.n };
            let mut v = vec![];
            for s in [p.s0, p.s1] {
                for w in [p.w0, p.w1] {
                    for y in [p.y0, p.y1] {
                        v.push(e.p(s, y, w));
                    }
                }
            }
            v
        }
    }
}

/// One street face and what already stands out of it.
struct Face<'a> {
    edge: &'a Edge,
    e: Eb,
    /// The openings recorded on this face (y absolute).
    openings: Vec<&'a OpeningRecord>,
    /// Every opening's and glazing part's rectangle on the face (s0, s1, y0,
    /// y1): ribbon and shop glazing is not always recorded as an opening.
    apertures: Vec<[f64; 4]>,
    /// Plan parts standing out of the face (in its basis), and what this
    /// module placed on it so far.
    obstacles: Vec<Bx>,
    placed: Vec<Bx>,
    /// The most anything may stand out of it.
    reach: f64,
    /// Existing fire escape on this face (the port's): (s0, s1, projection, deck tops).
    port_escape: Option<(f64, f64, f64, Vec<f64>)>,
    shop: bool,
}

impl Face<'_> {
    fn free(&self, b: &Bx) -> bool {
        b.w1 <= self.reach + 1e-9 && b.s0 >= -1e-9 && b.s1 <= self.edge.length + 1e-9 && !self.obstacles.iter().chain(&self.placed).any(|o| o.hits(b, 0.004))
    }
    /// The furthest any obstacle stands out over the rectangle (s0..s1, y0..y1).
    fn standout(&self, s0: f64, s1: f64, y0: f64, y1: f64) -> f64 {
        let q = Bx::new(s0, s1, y0, y1, -1.0, 10.0);
        self.obstacles.iter().chain(&self.placed).filter(|o| o.hits(&q, 0.002)).map(|o| o.w1).fold(0.0, f64::max)
    }
    /// The (s0, s1) wall spans of storey band (y0, y1) with no opening or glazing.
    fn wall_spans(&self, y0: f64, y1: f64, pad: f64) -> Vec<(f64, f64)> {
        let mut cuts: Vec<(f64, f64)> = self.apertures.iter().filter(|o| o[3] > y0 && o[2] < y1).map(|o| (o[0] - pad, o[1] + pad)).collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = vec![];
        let mut at = 0.0f64;
        for (a, b) in cuts {
            if a > at {
                out.push((at, a));
            }
            at = at.max(b);
        }
        if at < self.edge.length {
            out.push((at, self.edge.length));
        }
        out
    }
}

/// Is a part the port's fire escape?
fn is_escape_part(kind: &str) -> bool {
    kind.starts_with("fire-escape")
}

fn faces<'a>(planned: &'a Planned, l: &Limits) -> Vec<Face<'a>> {
    let frame = &planned.frame;
    let plan = &planned.plan;
    let front_ok = matches!(planned.front.source, "street" | "service") && planned.front.occluded != Some(true);
    let alley = planned.front.source == "service";
    let shop_edges: Vec<usize> = plan
        .extra
        .get("shopGround")
        .and_then(|g| g.get("edges"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).map(|v| v as usize).collect())
        .unwrap_or_default();
    let mut out = vec![];
    for edge in &frame.edges {
        let street = (edge.index == planned.front.index && front_ok) || edge.street_return;
        if !street || edge.party || edge.lot_line.is_some() || edge.length < 3.0 {
            continue;
        }
        let e = Eb::of(edge);
        let mut obstacles = vec![];
        let mut port: Option<(f64, f64, f64, Vec<f64>)> = None;
        for p in plan.parts.iter().chain(&plan.stoop_parts) {
            if matches!(p.shape, Shape::Prism(_)) && p.kind == "roof-slab" {
                continue;
            }
            if p.edge == Some(edge.index) && is_escape_part(&p.kind) {
                if p.kind == "fire-escape" {
                    let q = port.get_or_insert((f64::MAX, f64::MIN, 0.0, vec![]));
                    q.0 = q.0.min(p.s0);
                    q.1 = q.1.max(p.s1);
                    q.2 = q.2.max(p.w1);
                    q.3.push(p.y1);
                }
                continue;
            }
            let (mut s0, mut s1, mut w0, mut w1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            if p.edge == Some(edge.index) && !matches!(p.shape, Shape::Prism(_)) {
                (s0, s1, w0, w1, y0, y1) = (p.s0, p.s1, p.w0, p.w1, p.y0, p.y1);
            } else {
                for c in part_corners(p) {
                    let (s, w) = e.local(c[0], c[2]);
                    s0 = s0.min(s);
                    s1 = s1.max(s);
                    w0 = w0.min(w);
                    w1 = w1.max(w);
                    y0 = y0.min(c[1]);
                    y1 = y1.max(c[1]);
                }
            }
            // what stands out of this face, near it.
            if w1 <= 0.012 || w0 > 3.0 || s1 < -1.0 || s0 > edge.length + 1.0 {
                continue;
            }
            obstacles.push(Bx::new(s0, s1, y0, y1, w0.max(0.0), w1));
        }
        if let Some(p) = port.as_mut() {
            p.3.sort_by(|a, b| a.total_cmp(b));
            p.3.dedup_by(|a, b| (*a - *b).abs() < 0.05);
        }
        let reach = if !l.read { 0.0 } else if alley && edge.index == planned.front.index { l.alley_reach_m } else { l.street_reach_m };
        let openings: Vec<&OpeningRecord> = plan.edges.iter().filter(|r| r.index == edge.index).flat_map(|r| r.openings.iter()).collect();
        let mut apertures: Vec<[f64; 4]> = openings.iter().map(|o| [o.s0, o.s1, o.y0, o.y1]).collect();
        apertures.extend(plan.parts.iter().filter(|p| p.edge == Some(edge.index) && crate::pipeline::is_opening_part(p)).map(|p| [p.s0, p.s1, p.y0, p.y1]));
        out.push(Face { edge, e, openings, apertures, obstacles, placed: vec![], reach, port_escape: port, shop: shop_edges.contains(&edge.index) });
    }
    out
}

// ---------------------------------------------------------------------------
// The elements.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Escape {
    face: usize,
    s0: f64,
    s1: f64,
    /// Off the wall (clears the bands and sills), and the landing depth.
    w0: f64,
    depth: f64,
    /// Deck tops, ascending (absolute y).
    decks: Vec<f64>,
    /// Where each deck's knee brackets meet the wall: off it by what they
    /// pass (a string course, a sill band), as bolted over it.
    knee_w: Vec<f64>,
    /// Stair foot at the s0 end (true) or the s1 end of each flight.
    flights_from_s0: Vec<bool>,
    ladder: Option<(f64, f64)>, // (s centre, foot y)
    roof_ladder: Option<(f64, f64)>, // (s centre, top y)
    paint: [u8; 3],
    rust: f64,
    seed: u32,
}

#[derive(Clone, Debug)]
struct Awning {
    face: usize,
    s0: f64,
    s1: f64,
    top: f64,
    proj: f64,
    fall: f64,
    valance: f64,
    form: u8, // 0 shed, 1 dome, 2 box, 3 open retractable
    colour: [u8; 3],
    stripe: Option<([u8; 3], f64)>,
    torn: Option<(f64, f64)>, // (s at, width)
    closed_sides: bool,
}

#[derive(Clone, Debug)]
struct Shape2 {
    kind: u8, // 0 disc, 1 ring, 2 diamond, 3 band, 4 rect, 5 half disc
    c: [f64; 2],
    r: [f64; 2],
    colour: [u8; 3],
}

#[derive(Clone, Debug)]
struct SignFace {
    face: usize,
    s0: f64,
    s1: f64,
    y0: f64,
    y1: f64,
    w: f64,
    field: [u8; 3],
    shapes: Vec<Shape2>,
    glow: u8,
    /// Its words (stage S1; None: the colour fields and shapes alone).
    letters: Option<Lettering>,
}

#[derive(Clone, Debug)]
struct Blade {
    face: usize,
    s: f64,
    y0: f64,
    y1: f64,
    w0: f64,
    w1: f64,
    neon: bool,
    field: [u8; 3],
    shapes: Vec<Shape2>,
    tubes: [u8; 3],
    glow: u8,
    /// Its words (stage S1; None: the shapes or the tube emblems alone).
    letters: Option<Lettering>,
}

/// (stage S1) A small neon sign hung in a shop window (OPEN, or the trade's
/// word) on its dark backing, just in front of the pane.
#[derive(Clone, Debug)]
struct WindowSign {
    face: usize,
    s0: f64,
    s1: f64,
    y0: f64,
    y1: f64,
    /// Its back (in front of the glass, inside the window's reveal).
    w0: f64,
    letters: Lettering,
}

#[derive(Clone, Debug)]
struct Shutter {
    face: usize,
    s0: f64,
    s1: f64,
    floor: f64,
    head: f64,
    w0: f64,
    down: bool,
    colour: [u8; 3],
}

/// Security bars over a ground-floor window.
#[derive(Clone, Debug)]
struct Grille {
    face: usize,
    s0: f64,
    s1: f64,
    y0: f64,
    y1: f64,
    w0: f64,
    colour: [u8; 3],
}

#[derive(Clone, Debug)]
struct Boarding {
    face: usize,
    s0: f64,
    s1: f64,
    y0: f64,
    y1: f64,
    w0: f64,
}

/// A service fitting's paint: the wall's own (painted over with the
/// front), galvanised, or dark.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Paint {
    Wall,
    Galv,
    Dark,
    Colour([u8; 3]),
}

#[derive(Clone, Debug)]
struct Downspout {
    face: usize,
    s: f64,
    /// Off the wall (in front of the thin bands it passes).
    w0: f64,
    /// Runs of pipe (y0, y1) between the bands it passes.
    runs: Vec<(f64, f64)>,
    /// Where its straps are (clear of the bands it stands in front of).
    straps: Vec<f64>,
    head: Option<f64>,
    paint: Paint,
}

#[derive(Clone, Debug)]
struct Meters {
    face: usize,
    s: f64,
    floor: f64,
    w0: f64,
    electric: u32,
    gas: bool,
    conduit_top: f64,
    fdc: Option<(f64, [u8; 3])>, // (s, colour)
    paint: Paint,
}

#[derive(Clone, Debug)]
struct WindowAc {
    face: usize,
    s0: f64,
    s1: f64,
    sill: f64,
}

#[derive(Clone, Debug)]
struct Dish {
    at: V3,
    r: f64,
    /// Where it looks (unit, world).
    look: V3,
    /// On a front: the wall point its arm is fixed to (None: on the roof, on a mast).
    mount: Option<V3>,
}

#[derive(Clone, Debug)]
struct Scaffold {
    face: usize,
    s0: f64,
    s1: f64,
    floor: f64,
    top: f64,
    w0: f64,
}

#[derive(Clone, Debug)]
enum Roof {
    Chimney { c: [f64; 2], u: [f64; 2], half: [f64; 2], y0: f64, y1: f64, pots: u32 },
    Vent { c: [f64; 2], r: f64, y0: f64, y1: f64, cap: bool, dark: bool },
    Aerial { c: [f64; 2], y0: f64, h: f64, yaw: f64, elements: u32 },
}

/// What the planner placed on one building.
#[derive(Clone, Debug, Default)]
pub struct Census {
    pub fire_escapes: u32,
    pub landings: u32,
    pub replaced_escapes: u32,
    pub awnings: u32,
    pub sign_faces: u32,
    pub blade_signs: u32,
    /// (stage S1) sign faces and blade signs that carry words, and of them in neon.
    pub lettered: u32,
    pub neon_letters: u32,
    /// (stage S1) neon signs hung in shop windows.
    pub window_signs: u32,
    pub shutters: u32,
    pub boarded: u32,
    pub grilles: u32,
    pub downspouts: u32,
    pub meter_sets: u32,
    pub window_acs: u32,
    pub dishes: u32,
    pub scaffolds: u32,
    pub chimneys: u32,
    pub vents: u32,
    pub aerials: u32,
}

/// One building's clutter (planned once, drawn per rung).
#[derive(Clone, Debug, Default)]
pub struct ClutterPlan {
    escapes: Vec<Escape>,
    awnings: Vec<Awning>,
    faces_signs: Vec<SignFace>,
    blades: Vec<Blade>,
    window_signs: Vec<WindowSign>,
    shutters: Vec<Shutter>,
    grilles: Vec<Grille>,
    boards: Vec<Boarding>,
    downspouts: Vec<Downspout>,
    meters: Vec<Meters>,
    acs: Vec<WindowAc>,
    dishes: Vec<Dish>,
    scaffolds: Vec<Scaffold>,
    roof: Vec<Roof>,
    /// Faces' bases (by face index).
    bases: Vec<(usize, Eb)>,
    /// The port's escapes drawn here in their place (edge indices).
    pub replaced: Vec<usize>,
    pub census: Census,
    /// Every placed element's bounds: (kind, edge, box in that edge's basis)
    /// for street-face pieces (the tests check them against the plan's parts and each other).
    /// (the element's id: one element's pieces may touch each other.)
    pub boxes: Vec<(&'static str, usize, u32, Bx)>,
    /// Roof pieces' world bounds ([x0, y0, z0], [x1, y1, z1]).
    pub roof_boxes: Vec<(&'static str, V3, V3)>,
    ids: u32,
}

fn pick<T: Copy>(list: &[T], r: f64) -> T {
    list[((r * list.len() as f64) as usize).min(list.len() - 1)]
}

fn weighted<T: Copy>(list: &[(T, f64)], r: f64) -> T {
    let total: f64 = list.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    for (v, w) in list {
        acc += w / total;
        if r < acc {
            return *v;
        }
    }
    list[list.len() - 1].0
}

/// How often a district's older multi-unit fronts carry fire escapes
/// (share of qualifying fronts; Tenderloin, Chinatown and Nob Hill densest).
/// INFERRED from the districts' street fronts.
fn escape_share(district: &str) -> f64 {
    match district {
        "Tenderloin" => 0.8,
        "Chinatown" => 0.75,
        "Nob Hill" => 0.65,
        "South of Market" => 0.55,
        "Financial District/South Beach" => 0.45,
        "North Beach" | "Mission" => 0.45,
        "Russian Hill" | "Hayes Valley" | "Western Addition" => 0.35,
        "Haight Ashbury" | "Lone Mountain/USF" | "Japantown" => 0.28,
        "Castro/Upper Market" | "Inner Richmond" | "Inner Sunset" | "Potrero Hill" => 0.2,
        "Marina" | "Pacific Heights" | "Presidio Heights" => 0.12,
        _ => 0.1,
    }
}

/// Street-front clutter density by district (shutters, blades, meters):
/// denser on the old commercial streets.
fn street_density(district: &str) -> f64 {
    match district {
        "Tenderloin" | "Chinatown" | "Mission" => 1.45,
        "South of Market" | "North Beach" | "Castro/Upper Market" | "Excelsior" | "Outer Mission" => 1.2,
        "Marina" | "Pacific Heights" | "Presidio Heights" | "Seacliff" => 0.6,
        _ => 1.0,
    }
}

impl ClutterPlan {
    fn next_id(&mut self) -> u32 {
        self.ids += 1;
        self.ids
    }
    /// Record an element's pieces on its face (and in the census of boxes).
    fn put(&mut self, f: &mut Face, kind: &'static str, pieces: &[Bx]) {
        let id = self.next_id();
        for b in pieces {
            f.placed.push(*b);
            self.boxes.push((kind, f.edge.index, id, *b));
        }
    }
}

/// Plan a building's clutter at `level` (1 or 2; 0 plans nothing).
pub fn plan(planned: &Planned, level: u8) -> ClutterPlan {
    let mut out = ClutterPlan::default();
    if level == 0 {
        return out;
    }
    let l = limits();
    let c = &planned.classified;
    let plan = &planned.plan;
    let ap = building_appearance(&c.record_id, c, 1900.0);
    let civic = c.civic.as_ref();
    let district = civic.and_then(|v| v.district.as_deref()).unwrap_or("");
    let use_code = civic.and_then(|v| v.use_code.as_deref()).unwrap_or("");
    let year = ap.year_built;
    let storeys = plan.storeys.len();
    let base = plan.base_y;
    let Some(ground) = plan.storeys.first() else { return out };
    let floor = base + ground.y0;
    let seed = c.seed;
    let mut faces = faces(planned, l);
    out.bases = faces.iter().map(|f| (f.edge.index, f.e)).collect();
    let density = street_density(district);
    let multi = matches!(use_code, "MRES" | "COMR") || matches!(c.archetype, "bay-window-apartment" | "edwardian-flats" | "downtown-tower" | "retail-strip" | "soma-warehouse");

    // ---- fire escapes -----------------------------------------------------
    let detail_on = crate::typology::detail_enabled();
    let mut rng = Rng::new(&format!("{seed}|facade-clutter|escape"));
    let roll = rng.next();
    // (Edwardian flats and SoMa warehouses carry the port's own: drawn here in their place, none added.)
    let max_fe_storeys = if detail_on { 12 } else { 9 };
    let arch_k = match c.archetype {
        "bay-window-apartment" => 1.0,
        "downtown-tower" => {
            if storeys <= max_fe_storeys {
                0.7
            } else {
                0.0
            }
        }
        "retail-strip" => 0.6,
        _ => 0.0,
    };
    let max_fe_year = if detail_on { 1948.0 } else { 1936.0 };
    let fe_share = if detail_on { (escape_share(district) * 1.25).min(0.92) } else { escape_share(district) };
    let eligible = (3..=max_fe_storeys).contains(&storeys) && year < max_fe_year && multi && arch_k > 0.0;
    let wants = eligible && roll < fe_share * arch_k;
    for fi in 0..faces.len() {
        if let Some((s0, s1, proj, decks)) = faces[fi].port_escape.clone() {
            // the port's own fire escape, drawn here in its place (kept as it was if this one does not fit).
            let r = Rng::new(&format!("{seed}|facade-clutter|port-escape|{}", faces[fi].edge.index)).next();
            if let Some(esc) = fit_escape(&faces[fi], fi, s0, s1, proj.clamp(0.9, FE_DEPTH_M + 0.1), Some(decks.clone()), plan, &ap, r, seed) {
                out.replaced.push(faces[fi].edge.index);
                out.census.replaced_escapes += 1;
                out.put(&mut faces[fi], "fire-escape", &escape_boxes(&esc));
                out.census.fire_escapes += 1;
                out.census.landings += esc.decks.len() as u32;
                out.escapes.push(esc);
            } else {
                // the port's stays: keep its volume clear.
                let (a, b) = (s0, s1);
                let lo = decks.first().copied().unwrap_or(floor) - 2.5;
                let hi = decks.last().copied().unwrap_or(floor) + 1.2;
                faces[fi].obstacles.push(Bx::new(a, b, lo, hi, 0.0, proj));
            }
            continue;
        }
        if !wants {
            continue;
        }
        let max_here = if faces[fi].edge.length > 18.0 { 2 } else { 1 };
        let cols = window_columns(&faces[fi], 1);
        let len = faces[fi].edge.length;
        // prefer stacks away from the middle (the entrance) and the very ends, then a roll.
        let mut keyed: Vec<(f64, usize)> = (0..cols.len())
            .map(|k| {
                let t = (cols[k].0 + cols[k].1) / 2.0 / len;
                ((t - 0.3).abs().min((t - 0.7).abs()) + rng.next() * 0.25, k)
            })
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut placed_here = 0;
        for (_, k) in keyed {
            if placed_here >= max_here {
                break;
            }
            let (a, b) = cols[k];
            let mid = (a + b) / 2.0;
            let half = ((b - a) / 2.0 + 0.45).clamp(1.35, 2.3);
            let r = rng.next();
            if let Some(esc) = fit_escape(&faces[fi], fi, mid - half, mid + half, FE_DEPTH_M, None, plan, &ap, r, seed ^ (k as u32 + 1).wrapping_mul(0x9e37_79b9)) {
                out.put(&mut faces[fi], "fire-escape", &escape_boxes(&esc));
                out.census.fire_escapes += 1;
                out.census.landings += esc.decks.len() as u32;
                out.escapes.push(esc);
                placed_here += 1;
            }
        }
    }

    // ---- shop fronts: signs on the fascia boards, blade signs, awnings, shutters ----
    let shop_bays: Vec<serde_json::Value> = plan.extra.get("shopBays").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    // tenancies: (edge, tenancy, s0, s1, head, floor, shop type)
    let mut tenancies: Vec<(usize, i64, f64, f64, f64, f64, String)> = vec![];
    for b in &shop_bays {
        let (Some(edge), Some(t)) = (b.get("edge").and_then(|v| v.as_f64()), b.get("tenancy").and_then(|v| v.as_f64())) else { continue };
        let (s0, s1, y1) = (b["s0"].as_f64().unwrap_or(0.0), b["s1"].as_f64().unwrap_or(0.0), b["y1"].as_f64().unwrap_or(0.0));
        let fy = b["floorY"].as_f64().unwrap_or(floor);
        let ty = b["shopType"].as_str().unwrap_or("").to_string();
        match tenancies.iter_mut().find(|x| x.0 == edge as usize && x.1 == t as i64) {
            Some(x) => {
                x.2 = x.2.min(s0);
                x.3 = x.3.max(s1);
                x.4 = x.4.max(y1);
            }
            None => tenancies.push((edge as usize, t as i64, s0, s1, y1, fy, ty)),
        }
    }
    // (stage S1) each tenancy's fascia text (the shop identity's word, a name's words looked up only).
    let mut tenancy_text: HashMap<(usize, i64), String> = HashMap::new();
    for b in &shop_bays {
        if let (Some(edge), Some(t), Some(tx)) = (b.get("edge").and_then(|v| v.as_f64()), b.get("tenancy").and_then(|v| v.as_f64()), b.get("text").and_then(|v| v.as_str())) {
            tenancy_text.entry((edge as usize, t as i64)).or_insert_with(|| tx.to_string());
        }
    }
    let mut srng = Rng::new(&format!("{seed}|facade-clutter|shops"));
    // the fascia boards (the shop ground's signbands): a face of colour fields and shapes.
    let bands: Vec<&Part> = plan.parts.iter().filter(|p| p.kind == "signband" && p.is_box()).collect();
    for band in bands {
        let Some(fi) = faces.iter().position(|f| Some(f.edge.index) == band.edge) else { continue };
        let r = srng.next();
        let tn = band.meta.get("tenancy").and_then(|v| v.as_f64()).unwrap_or(-1.0) as i64;
        let key = hash_seed(&format!("{seed}|sign|{}|{tn}", faces[fi].edge.index));
        if r > 0.86 {
            continue;
        }
        if let Some(mut sf) = fit_sign_face(&faces[fi], fi, band, &mut srng, key) {
            // (stage S1) its words: the band's own text, else the shop under it, on a stream of
            // their own (the placement's draws stay as they were).
            let ei = faces[fi].edge.index;
            let under = tenancies.iter().filter(|t| t.0 == ei).max_by(|a, b| {
                let o = |t: &&(usize, i64, f64, f64, f64, f64, String)| t.3.min(sf.s1) - t.2.max(sf.s0);
                o(a).total_cmp(&o(b))
            });
            let text = band.meta.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()).or_else(|| under.and_then(|t| tenancy_text.get(&(t.0, t.1)).cloned()));
            let family = sign_family(text.as_deref(), under.map(|t| t.6.as_str()).unwrap_or(""));
            let e = &faces[fi].e;
            let right_s = if e.n[1] * e.u[0] - e.n[0] * e.u[1] >= 0.0 { 1.0 } else { -1.0 };
            let mut lrng = Rng::new(&format!("{seed}|facade-clutter|letters|{ei}|{tn}"));
            sf.letters = letter_fascia(&faces[fi], &sf, family, ap.paint, sf.glow != 0, key, right_s, &mut lrng);
            out.put(&mut faces[fi], "sign-face", &[Bx::new(sf.s0, sf.s1, sf.y0, sf.y1, sf.w, sf.w + FACE_PANEL_M)]);
            out.census.sign_faces += 1;
            out.census.lettered += sf.letters.is_some() as u32;
            out.census.neon_letters += sf.letters.as_ref().is_some_and(|l| l.code.mode == LETTER_NEON) as u32;
            out.faces_signs.push(sf);
        }
    }
    for t in &tenancies {
        let Some(fi) = faces.iter().position(|f| f.edge.index == t.0) else { continue };
        let (s0, s1, head, fy) = (t.2, t.3, t.4, t.5);
        let ty = t.6.as_str();
        let food = matches!(ty, "cafe" | "bar" | "bakery" | "restaurant" | "deli" | "diner" | "pizza" | "grocery");
        let key = hash_seed(&format!("{seed}|sign|{}|{}", t.0, t.1));
        let ei = faces[fi].edge.index;
        let has_shade = plan.parts.iter().any(|p| p.edge == Some(ei) && matches!(p.kind.as_str(), "awning" | "canopy") && p.s1 > s0 + 0.2 && p.s0 < s1 - 0.2);
        let has_blade = plan.parts.iter().any(|p| p.edge == Some(ei) && p.kind == "blade-sign" && p.s0 > s0 - 1.0 && p.s1 < s1 + 1.0);
        // a blade sign (Chinatown, the Mission and the Castro carry the most).
        let p_blade_base = if detail_on {
            if matches!(ty, "bar" | "restaurant" | "cafe" | "studio") { 0.68 } else { 0.46 }
        } else if matches!(ty, "bar" | "restaurant" | "cafe" | "studio") {
            0.42
        } else {
            0.24
        };
        let p_blade = p_blade_base * density;
        if !has_blade && srng.next() < p_blade {
            if let Some(mut bl) = fit_blade(&faces[fi], fi, s0, s1, plan, &mut srng, key) {
                // (stage S1) the shop's words on both faces, on a stream of their own.
                let family = sign_family(tenancy_text.get(&(t.0, t.1)).map(|s| s.as_str()), ty);
                let mut lrng = Rng::new(&format!("{seed}|facade-clutter|letters|blade|{}|{}", t.0, t.1));
                bl.letters = letter_blade(&bl, family, ap.paint, key, &mut lrng);
                out.put(&mut faces[fi], "blade-sign", &[blade_box(&bl)]);
                out.census.blade_signs += 1;
                out.census.lettered += bl.letters.is_some() as u32;
                out.census.neon_letters += bl.letters.as_ref().is_some_and(|l| l.code.mode == LETTER_NEON) as u32;
                out.blades.push(bl);
            }
        }
        // an awning where the shop has no shade.
        let p_awning = if detail_on {
            if food { 0.78 } else { 0.62 }
        } else if food {
            0.6
        } else {
            0.38
        };
        let r_aw = srng.next();
        let mut shaded = has_shade;
        if !has_shade && r_aw < p_awning {
            if let Some(aw) = fit_awning(&faces[fi], fi, s0, s1, head, fy, &ap, &mut srng) {
                out.put(&mut faces[fi], "awning", &[awning_box(&aw)]);
                out.census.awnings += 1;
                out.awnings.push(aw);
                shaded = true;
            }
        }
        // a roll-down shutter (or, level 2, plywood) where there is no shade.
        let p_shutter = 0.3 * density;
        let r_sh = srng.next();
        let r_board = srng.next();
        let down = srng.next() < if detail_on && (food || district.contains("Financial")) { 0.0 } else { 0.22 };
        let colour = pick(&[GALV, [120, 124, 126], [70, 74, 78], [150, 154, 152]], srng.next());
        if !shaded && level >= 2 && r_board < 0.018 * density {
            if let Some(bd) = fit_boarding(&faces[fi], fi, s0, s1, fy, head) {
                out.put(&mut faces[fi], "boarding", &[Bx::new(bd.s0, bd.s1, bd.y0, bd.y1, bd.w0, bd.w0 + 0.02)]);
                out.census.boarded += 1;
                out.boards.push(bd);
                continue;
            }
        }
        if !shaded && r_sh < p_shutter {
            if let Some(sh) = fit_shutter(&faces[fi], fi, s0, s1, fy, head, down, colour) {
                out.put(&mut faces[fi], "shutter", &shutter_boxes(&sh));
                out.census.shutters += 1;
                out.shutters.push(sh);
            }
        }
    }

    // ---- security bars on ground-floor windows (older buildings) -----------
    let mut grng = Rng::new(&format!("{seed}|facade-clutter|grilles"));
    let p_bars = match district {
        "Mission" | "Tenderloin" | "Excelsior" | "Bayview Hunters Point" | "Outer Mission" | "Chinatown" | "Visitacion Valley" | "Portola" => 0.55,
        "Marina" | "Pacific Heights" | "Presidio Heights" | "Seacliff" | "Russian Hill" => 0.08,
        _ => 0.25,
    };
    let bars_roll = grng.next();
    let bars_colour = weighted(&[([28u8, 28, 28], 0.7), ([226, 224, 216], 0.2), ([34, 52, 42], 0.1)], grng.next());
    if year < 1975.0 && bars_roll < p_bars {
        for fi in 0..faces.len() {
            let wins: Vec<(f64, f64, f64, f64)> = faces[fi]
                .openings
                .iter()
                .filter(|o| o.storey == 0 && o.type_ == "punched" && o.s1 - o.s0 >= 0.45 && o.s1 - o.s0 <= 2.8 && o.y1 - o.y0 >= 0.7)
                .map(|o| (o.s0, o.s1, o.y0, o.y1))
                .collect();
            for (s0, s1, y0, y1) in wins {
                if let Some(g) = fit_grille(&faces[fi], fi, s0, s1, y0, y1, bars_colour) {
                    out.put(&mut faces[fi], "grille", &[Bx::new(g.s0, g.s1, g.y0, g.y1, g.w0, g.w0 + 0.026)]);
                    out.census.grilles += 1;
                    out.grilles.push(g);
                }
            }
        }
    }

    // ---- downspouts --------------------------------------------------------
    let has_pipes = |ei: usize| plan.parts.iter().any(|p| p.edge == Some(ei) && p.kind == "downpipe");
    let pitched = plan.parts.iter().any(|p| matches!(p.kind.as_str(), "roof-slope" | "roof-gable" | "gable" | "hip" | "pitch"));
    let mut drng = Rng::new(&format!("{seed}|facade-clutter|downspouts"));
    for fi in 0..faces.len() {
        let ei = faces[fi].edge.index;
        if has_pipes(ei) || pitched || storeys < 2 {
            continue;
        }
        let edge = faces[fi].edge;
        let frame = &planned.frame;
        let next_party = frame.edges.get(edge.next_index).map(|e| e.party || e.lot_line.is_some()).unwrap_or(false);
        let prev_party = frame.edges.get(edge.prev_index).map(|e| e.party || e.lot_line.is_some()).unwrap_or(false);
        let both = drng.next() < 0.3;
        let finish_roll = drng.next();
        let lone = drng.next();
        let lone_end = drng.next() < 0.5;
        let mut ends = vec![];
        if prev_party || both {
            ends.push(false);
        }
        if next_party || both {
            ends.push(true);
        }
        if ends.is_empty() && lone < 0.5 {
            ends.push(lone_end);
        }
        for at_end in ends {
            // painted over with the front, galvanised, or dark.
            let paint = if finish_roll < 0.45 {
                Paint::Wall
            } else if finish_roll < 0.8 {
                Paint::Galv
            } else {
                Paint::Dark
            };
            if let Some(ds) = fit_downspout(&faces[fi], fi, at_end, floor, plan, paint) {
                let pieces: Vec<Bx> = downspout_boxes(&ds);
                out.put(&mut faces[fi], "downspout", &pieces);
                out.census.downspouts += 1;
                out.downspouts.push(ds);
            }
        }
    }

    // ---- meters, conduit, the standpipe connection -----------------------
    let mut mrng = Rng::new(&format!("{seed}|facade-clutter|meters"));
    for fi in 0..faces.len() {
        let go = mrng.next();
        let e_roll = mrng.next();
        let gas = mrng.next() < 0.7;
        let fdc = storeys >= 4 && mrng.next() < 0.85;
        let paint = if mrng.next() < 0.3 { Paint::Wall } else { Paint::Colour(METER_GREY) };
        if !(faces[fi].edge.index == planned.front.index || faces[fi].shop) || go > (0.7 * density).min(0.95) {
            continue;
        }
        let units = if multi { (storeys as u32).clamp(2, 6) } else { 1 };
        let electric = (1 + (e_roll * units as f64) as u32).min(units);
        if let Some(m) = fit_meters(&faces[fi], fi, floor, electric, gas, fdc, &mut mrng, l, paint) {
            out.put(&mut faces[fi], "meters", &meter_boxes(&m));
            out.census.meter_sets += 1;
            out.meters.push(m);
        }
    }

    // ---- level 2: air conditioners, front dishes, scaffolding -------------
    if level >= 2 {
        let mut arng = Rng::new(&format!("{seed}|facade-clutter|level2"));
        for fi in 0..faces.len() {
            // window air conditioners: rare (San Francisco's mild climate), older apartments.
            if multi && year < 1960.0 && storeys >= 3 {
                let cands: Vec<(f64, f64, f64)> =
                    faces[fi].openings.iter().filter(|o| o.storey >= 1 && o.type_ == "punched" && o.s1 - o.s0 >= 0.75 && o.s1 - o.s0 <= 2.6).map(|o| (o.s0, o.s1, o.y0)).collect();
                for (s0, s1, sill) in cands {
                    if arng.next() < 0.012 {
                        let mid = (s0 + s1) / 2.0;
                        let ac = WindowAc { face: fi, s0: mid - 0.33, s1: mid + 0.33, sill };
                        let b = Bx::new(ac.s0, ac.s1, sill + 0.002, sill + 0.4, 0.0, 0.42);
                        let bracket = Bx::new(ac.s0, ac.s1, sill - 0.36, sill, 0.1, 0.42);
                        if faces[fi].free(&b) && faces[fi].free(&bracket) && ac.s0 > s0 && ac.s1 < s1 {
                            out.put(&mut faces[fi], "window-ac", &[b, bracket]);
                            out.census.window_acs += 1;
                            out.acs.push(ac);
                        }
                    }
                }
            }
            // a dish on a front pier of an apartment block facing the satellites.
            let r_dish = arng.next();
            let r_st = arng.next();
            let r_span = arng.next();
            let e = faces[fi].e;
            let toward = dot3(e.dn(), dish_aim());
            if multi && storeys >= 3 && toward > 0.2 && r_dish < 0.1 * density {
                let st = 1 + (r_st * (storeys - 1) as f64) as usize;
                let (y0, y1) = (base + plan.storeys[st].y0, base + plan.storeys[st].y1);
                let spans: Vec<(f64, f64)> = faces[fi].wall_spans(y0, y1, 0.08).into_iter().filter(|(a, b)| b - a >= 0.9).collect();
                if !spans.is_empty() {
                    let (a, b) = spans[((r_span * spans.len() as f64) as usize).min(spans.len() - 1)];
                    let s = (a + b) / 2.0;
                    let y = y0 + (y1 - y0) * 0.55;
                    let bb = Bx::new(s - 0.42, s + 0.42, y - 0.45, y + 0.42, 0.0, 0.72);
                    if faces[fi].free(&bb) {
                        out.put(&mut faces[fi], "dish", &[bb]);
                        out.dishes.push(Dish { at: e.p(s, y, 0.45), r: 0.3, look: dish_aim(), mount: Some(e.p(s, y - 0.2, 0.0)) });
                        out.census.dishes += 1;
                    }
                }
            }
        }
        // pipe scaffolding on a building under work (rare).
        if storeys >= 2 && arng.next() < 0.009 {
            if let Some(fi) = faces.iter().position(|f| f.edge.index == planned.front.index) {
                if let Some(sc) = fit_scaffold(&faces[fi], fi, floor, base + plan.envelope.wall_top) {
                    out.put(&mut faces[fi], "scaffold", &[Bx::new(sc.s0, sc.s1, sc.floor, sc.top, sc.w0, sc.w0 + 1.1)]);
                    out.census.scaffolds += 1;
                    out.scaffolds.push(sc);
                }
            }
        }
    }

    // ---- (stage S1) neon signs hung in shop windows: OPEN, or the trade's word ----
    // planned after everything else, on streams of their own, behind the face:
    // nothing placed before moves.
    for t in &tenancies {
        let Some(fi) = faces.iter().position(|f| f.edge.index == t.0) else { continue };
        let family = sign_family(tenancy_text.get(&(t.0, t.1)).map(|s| s.as_str()), t.6.as_str());
        let key = hash_seed(&format!("{seed}|sign|window|{}|{}", t.0, t.1));
        let mut wr = Rng::new(&format!("{seed}|facade-clutter|letters|window|{}|{}", t.0, t.1));
        if let Some(ws) = fit_window_sign(&faces[fi], fi, plan, t.2, t.3, family, density, key, &mut wr) {
            out.put(&mut faces[fi], "window-sign", &[Bx::new(ws.s0, ws.s1, ws.y0, ws.y1, ws.w0, ws.w0 + WINDOW_SIGN_M)]);
            out.census.window_signs += 1;
            out.census.lettered += 1;
            out.census.neon_letters += 1;
            out.window_signs.push(ws);
        }
    }

    // ---- roofs ---------------------------------------------------------------
    if !pitched {
        plan_roof(planned, &ap, year, multi, district, &mut out);
    }
    out
}

/// (stage S1) A window neon sign's backing board thickness. INFERRED.
const WINDOW_SIGN_M: f64 = 0.012;

/// (stage S1) Trades whose windows carry neon (bars, liquor, late food,
/// laundromats, nail salons): their phrases, and the share of them that hang one.
fn window_neon_phrase(p: &str) -> bool {
    neon_phrase(p) || matches!(p, "NAILS" | "NAIL SALON" | "LAUNDRY" | "COIN LAUNDRY" | "LAUNDROMAT" | "CAFE" | "COFFEE" | "DELI" | "TACOS" | "TAQUERIA" | "NOODLES")
}

/// (stage S1) A neon sign in the widest ground-floor pane of a tenancy
/// (s0..s1): high in the window, just in front of the glass; OPEN on most,
/// the trade's word in the rest.
#[allow(clippy::too_many_arguments)]
fn fit_window_sign(f: &Face, fi: usize, plan: &crate::elevation::Plan, s0: f64, s1: f64, family: &[&'static str], density: f64, key: u32, r: &mut Rng) -> Option<WindowSign> {
    let neon_trade = family.iter().any(|p| window_neon_phrase(p));
    let roll = r.next();
    let open_roll = r.next();
    let face_roll = r.next();
    let size_roll = r.next();
    let at_roll = r.next();
    let ink_roll = r.next();
    let border_roll = r.next();
    let win_sign_p = if crate::typology::detail_enabled() {
        if neon_trade { 0.56 } else { 0.22 }
    } else if neon_trade {
        0.28
    } else {
        0.06
    };
    if roll >= win_sign_p * density.min(1.3) {
        return None;
    }
    let ei = f.edge.index;
    let pane = plan
        .parts
        .iter()
        .filter(|q| q.edge == Some(ei) && q.kind == "glass" && q.is_box() && q.s0 >= s0 - 0.05 && q.s1 <= s1 + 0.05 && q.s1 - q.s0 >= 0.9 && q.y1 - q.y0 >= 1.2)
        .max_by(|a, b| (a.s1 - a.s0).total_cmp(&(b.s1 - b.s0)).then(b.s0.total_cmp(&a.s0)))?;
    let words: Vec<&'static str> = family.iter().copied().filter(|p| window_neon_phrase(p)).collect();
    let phrase = if open_roll < 0.58 || words.is_empty() { "OPEN" } else { pick(&words, open_roll) };
    let cells = cells_of(phrase, false);
    let cell = *cells.get(((face_roll * cells.len() as f64) as usize).min(cells.len().saturating_sub(1)))?;
    let aspect = letter_cells()[cell as usize].aspect;
    // letters a hand high (the common shop window sign is about 2 ft by 1 ft), within the pane.
    let border = border_roll < 0.8;
    let mut h = 0.17 + 0.07 * size_roll;
    let margin = if border { 0.7 } else { 0.3 };
    let bh = h * if border { 1.55 } else { 1.25 };
    let top = pane.y1 - 0.12 - 0.25 * size_roll;
    let (wy0, wy1) = (top - bh, top);
    let w0 = pane.w1 + 0.006;
    if wy0 < pane.y0 + 0.4 {
        return None;
    }
    // the widest span of the pane no mullion or bar in front of the glass crosses at the sign's height.
    let mut cuts: Vec<(f64, f64)> = plan
        .parts
        .iter()
        .filter(|q| q.edge == Some(ei) && q.kind != "glass" && q.is_box() && q.w1 > w0 - 0.001 && q.w0 < w0 + WINDOW_SIGN_M + 0.004 && q.y1 > wy0 && q.y0 < wy1 && q.s1 > pane.s0 && q.s0 < pane.s1)
        .map(|q| (q.s0, q.s1))
        .collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut best, mut at) = ((pane.s0, pane.s0), pane.s0);
    for (a, b) in cuts.iter().copied().chain([(pane.s1, pane.s1)]) {
        if a - at > best.1 - best.0 {
            best = (at, a);
        }
        at = at.max(b);
    }
    let room = (best.1 - best.0) - 0.16;
    if (aspect + margin) * h > room {
        h = room / (aspect + margin);
    }
    if h < 0.1 {
        return None;
    }
    let bw = (aspect + margin) * h;
    let bh = h * if border { 1.55 } else { 1.25 };
    let (wy0, wy1) = (top - bh, top);
    let slack = room - bw;
    let sc = best.0 + 0.08 + bw / 2.0 + slack * (0.5 + (at_roll - 0.5) * 0.8);
    let (ws0, ws1) = (sc - bw / 2.0, sc + bw / 2.0);
    let ink = weighted(&[(INK_PINK, 0.3), (INK_RED, 0.3), (INK_BLUE, 0.2), (INK_GOLD, 0.1), (INK_CREAM, 0.1)], ink_roll);
    let code = LetterCode { cell, mode: LETTER_NEON, query: 0, ink, field: FIELD_NEON_BOARD, emblem: if border { EMBLEM_BORDER } else { EMBLEM_NONE } };
    let letters = Lettering { code, aspect, height: h, centre: [sc, (wy0 + wy1) / 2.0], layout: 0, glow: glow_code(true, key) };
    Some(WindowSign { face: fi, s0: ws0, s1: ws1, y0: wy0, y1: wy1, w0, letters })
}

/// The window stacks of a face from storey `from` up: (s0, s1) of openings
/// that line up on (nearly) every upper storey.
fn window_columns(f: &Face, from: usize) -> Vec<(f64, f64)> {
    let mut by: Vec<(f64, f64, Vec<usize>)> = vec![];
    for o in &f.openings {
        if o.storey < from || matches!(o.type_.as_str(), "storefront" | "entrance" | "roller") {
            continue;
        }
        match by.iter_mut().find(|c| (c.0 - o.s0).abs() < 0.3 && (c.1 - o.s1).abs() < 0.3) {
            Some(c) => {
                if !c.2.contains(&o.storey) {
                    c.2.push(o.storey);
                }
            }
            None => by.push((o.s0, o.s1, vec![o.storey])),
        }
    }
    let top = by.iter().map(|c| c.2.len()).max().unwrap_or(0);
    by.into_iter().filter(|c| c.2.len() + 1 >= top && c.2.len() >= 2).map(|c| (c.0, c.1)).collect()
}

/// The stair flight from deck `k`: (foot s, head s).
fn flight(esc: &Escape, k: usize) -> (f64, f64) {
    let run = (esc.decks[k + 1] - esc.decks[k]) * FE_STAIR_RUN_PER_RISE;
    if esc.flights_from_s0[k] { (esc.s0 + 0.3, esc.s0 + 0.3 + run) } else { (esc.s1 - 0.3, esc.s1 - 0.3 - run) }
}

/// The pieces of an escape as boxes in its face's basis (what is checked).
fn escape_boxes(esc: &Escape) -> Vec<Bx> {
    let (w0, w1) = (esc.w0, esc.w0 + esc.depth);
    let mut v = vec![];
    for (k, &d) in esc.decks.iter().enumerate() {
        // the deck and its railing.
        v.push(Bx::new(esc.s0, esc.s1, d - 0.08, d + FE_RAIL_M + 0.01, w0, w1));
        // the knee brackets under it, at both ends.
        let kw = esc.knee_w.get(k).copied().unwrap_or(0.0);
        for bs in [esc.s0 + 0.08, esc.s1 - 0.08] {
            v.push(Bx::new(bs - 0.04, bs + 0.04, d - 0.62, d - 0.07, kw, w1 - 0.08));
        }
        if k + 1 < esc.decks.len() {
            // the stair to the next deck, in the outer half.
            let (a, b) = flight(esc, k);
            v.push(Bx::new(a - 0.1, b + 0.1, d, esc.decks[k + 1] + 0.9, w1 - FE_STAIR_W_M - 0.08, w1));
        }
    }
    if let Some((s, foot)) = esc.ladder {
        // the drop ladder hangs outside the lowest rail.
        v.push(Bx::new(s - FE_LADDER_W_M / 2.0 - 0.02, s + FE_LADDER_W_M / 2.0 + 0.02, foot, esc.decks[0] + FE_RAIL_M, w1 + 0.005, w1 + 0.08));
    }
    if let Some((s, top)) = esc.roof_ladder {
        let d = *esc.decks.last().unwrap();
        v.push(Bx::new(s - 0.25, s + 0.25, d, top, w1 - 0.55, w1 - 0.1));
        // the gooseneck over the parapet.
        v.push(Bx::new(s - 0.25, s + 0.25, top - 0.62, top + 0.02, -0.16, w1 - 0.1));
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn fit_escape(
    f: &Face,
    fi: usize,
    s0: f64,
    s1: f64,
    depth: f64,
    port_decks: Option<Vec<f64>>,
    plan: &crate::elevation::Plan,
    ap: &crate::appearance::Appearance,
    roll: f64,
    seed: u32,
) -> Option<Escape> {
    if f.reach < depth + 0.2 {
        return None;
    }
    let (mut s0, mut s1) = (s0.max(0.3), s1.min(f.edge.length - 0.3));
    let base = plan.base_y;
    let storeys = &plan.storeys;
    // deck tops: just under each upper storey's window sills, or the port's.
    let mut decks: Vec<f64> = match port_decks {
        Some(d) => d,
        None => {
            let mut d = vec![];
            for st in storeys.iter().skip(1) {
                let sill = f.openings.iter().filter(|o| o.storey == st.index && o.s1 > s0 && o.s0 < s1).map(|o| o.y0).fold(f64::INFINITY, f64::min);
                let floor_y = base + st.y0;
                d.push(if sill.is_finite() { (sill - 0.62).clamp(floor_y - 0.08, floor_y + 0.45) } else { floor_y + 0.05 });
            }
            d
        }
    };
    if decks.len() < 2 {
        return None;
    }
    // the stair and a foot landing must fit along the deck.
    let storey_h = (decks[1] - decks[0]).max(2.2);
    let need = storey_h * FE_STAIR_RUN_PER_RISE + 0.85;
    if s1 - s0 < need {
        let grow = (need - (s1 - s0)) / 2.0 + 0.01;
        s0 = (s0 - grow).max(0.3);
        s1 = (s1 + grow).min(f.edge.length - 0.3);
        if s1 - s0 < need {
            return None;
        }
    }
    // how far off the wall each deck must stand to clear its bands and sills;
    // decks are dropped from the top where a cornice or bay is in the way.
    let max_standoff = if crate::typology::detail_enabled() { 0.36 } else { FE_MAX_STANDOFF_M };
    let deck_stand = |d: f64| f.standout(s0, s1, d - 0.09, d + FE_RAIL_M + 0.02);
    while decks.len() >= 2 && deck_stand(*decks.last().unwrap()) > max_standoff {
        decks.pop();
    }
    if decks.len() < 2 || decks.iter().any(|d| deck_stand(*d) > max_standoff) {
        return None;
    }
    let w0 = decks.iter().map(|d| deck_stand(*d)).fold(0.0, f64::max) + 0.03;
    if w0 + depth > f.reach {
        return None;
    }
    let mut r = Rng::from_hash(seed ^ 0x51ed_270b);
    let paint = weighted(&FE_PAINTS, roll);
    // rust where the paint has gone (by the building's wear): patches, not the whole escape.
    let rust = (0.03 + ap.wear * 0.3 + r.next() * 0.08).min(0.35);
    let start_s0 = r.next() < 0.5;
    let flights: Vec<bool> = (0..decks.len() - 1).map(|k| (k % 2 == 0) == start_s0).collect();
    // the brackets stand on whatever band they pass (bolted over it), up to the deck's own standoff.
    let knee_w: Vec<f64> = decks.iter().map(|d| {
        let a = f.standout(s0 + 0.04, s0 + 0.12, d - 0.62, d - 0.07);
        let b = f.standout(s1 - 0.12, s1 - 0.04, d - 0.62, d - 0.07);
        a.max(b).min(w0)
    }).collect();
    let mut esc = Escape { face: fi, s0, s1, w0, depth, decks, knee_w, flights_from_s0: flights, ladder: None, roof_ladder: None, paint, rust, seed };
    // (the stair boxes reach 0.9 m over the upper deck: its handrail.)
    if !escape_boxes(&esc).iter().all(|b| f.free(b)) {
        return None;
    }
    // the drop ladder outside the lowest rail, at an end clear of what is under it.
    let d0 = esc.decks[0];
    let floor = base + storeys[0].y0;
    let foot = (d0 - FE_LADDER_LEN_M).max(floor + FE_LADDER_FOOT_M);
    if d0 - foot >= 0.6 && esc.w0 + esc.depth + 0.1 <= f.reach {
        let first = esc.flights_from_s0[0];
        for s in [if first { s0 + 0.45 } else { s1 - 0.45 }, if first { s1 - 0.45 } else { s0 + 0.45 }] {
            esc.ladder = Some((s, foot));
            if escape_boxes(&esc).last().map(|b| f.free(b)).unwrap_or(false) {
                break;
            }
            esc.ladder = None;
        }
    }
    // the gooseneck ladder over the parapet, where the top deck reaches it.
    let roof_top = base + plan.envelope.roof_top;
    let top = *esc.decks.last().unwrap();
    if roof_top - top <= 3.6 && roof_top - top > 1.2 {
        let end_s = if *esc.flights_from_s0.last().unwrap() { s1 - 0.35 } else { s0 + 0.35 };
        esc.roof_ladder = Some((end_s, roof_top + 0.9));
        let bs = escape_boxes(&esc);
        if !bs[bs.len() - 2..].iter().all(|b| f.free(b)) {
            esc.roof_ladder = None;
        }
    }
    Some(esc)
}

fn sign_design(r: &mut Rng, w: f64, h: f64, field: [u8; 3]) -> Vec<Shape2> {
    let contrast = |r: &mut Rng| {
        for _ in 0..24 {
            let c = pick(&SIGN_FIELDS, r.next());
            let d: i32 = (0..3).map(|k| (c[k] as i32 - field[k] as i32).abs()).sum();
            if d > 160 {
                return c;
            }
        }
        if field[0] as u32 + field[1] as u32 + field[2] as u32 > 380 { [30, 30, 30] } else { [222, 214, 196] }
    };
    let mut v = vec![];
    let a = contrast(r);
    match (r.next() * 5.0) as u32 {
        0 => {
            // a disc emblem at one end and a band along the rest.
            let rr = (h * 0.36).min(w * 0.2);
            let left = r.next() < 0.5;
            let cx = if left { -w / 2.0 + rr + h * 0.14 } else { w / 2.0 - rr - h * 0.14 };
            v.push(Shape2 { kind: 0, c: [cx, 0.0], r: [rr, rr], colour: a });
            let bw = w - 2.0 * rr - h * 0.5;
            if bw > 0.2 {
                let bx = if left { cx + rr + h * 0.12 + bw / 2.0 } else { cx - rr - h * 0.12 - bw / 2.0 };
                v.push(Shape2 { kind: 3, c: [bx, 0.0], r: [bw / 2.0, h * 0.09], colour: a });
            }
        }
        1 => {
            // two bands, top and bottom.
            v.push(Shape2 { kind: 3, c: [0.0, h * 0.32], r: [w * 0.46, h * 0.07], colour: a });
            v.push(Shape2 { kind: 3, c: [0.0, -h * 0.32], r: [w * 0.46, h * 0.07], colour: a });
        }
        2 => {
            // a ring emblem in the middle, flanked by short bands.
            let rr = (h * 0.36).min(w * 0.2);
            v.push(Shape2 { kind: 1, c: [0.0, 0.0], r: [rr, rr * 0.62], colour: a });
            let bw = (w / 2.0 - rr - h * 0.2) * 0.8;
            if bw > 0.1 {
                v.push(Shape2 { kind: 3, c: [-(rr + h * 0.15 + bw / 2.0), 0.0], r: [bw / 2.0, h * 0.06], colour: a });
                v.push(Shape2 { kind: 3, c: [rr + h * 0.15 + bw / 2.0, 0.0], r: [bw / 2.0, h * 0.06], colour: a });
            }
        }
        3 => {
            // diamonds in a row.
            let n = ((w / (h * 0.9)) as usize).clamp(1, 5);
            let step = w / n as f64;
            for i in 0..n {
                let cx = -w / 2.0 + step * (i as f64 + 0.5);
                v.push(Shape2 { kind: 2, c: [cx, 0.0], r: [(h * 0.3).min(step * 0.35), h * 0.3], colour: a });
            }
        }
        _ => {
            // a field block at one end and a half disc (a rising sun).
            let bw = (w * 0.28).min(h * 1.2);
            let left = r.next() < 0.5;
            let cx = if left { -w / 2.0 + bw / 2.0 + h * 0.1 } else { w / 2.0 - bw / 2.0 - h * 0.1 };
            v.push(Shape2 { kind: 4, c: [cx, 0.0], r: [bw / 2.0, h * 0.34], colour: a });
            let b = contrast(r);
            v.push(Shape2 { kind: 5, c: [-cx * 0.35, -h * 0.2], r: [h * 0.42, h * 0.42], colour: b });
        }
    }
    v
}

fn fit_sign_face(f: &Face, fi: usize, band: &Part, r: &mut Rng, key: u32) -> Option<SignFace> {
    let w = band.w1 + 0.001;
    let (y0, y1) = (band.y0 + 0.05, band.y1 - 0.05);
    let inset = r.next() * 0.25;
    let field = pick(&SIGN_FIELDS, r.next());
    let lit_share = if crate::typology::detail_enabled() { 0.86 } else { 0.72 };
    let lit = r.next() < lit_share;
    if y1 - y0 < 0.22 {
        return None;
    }
    // clear of the shop piers and lamps standing over the board.
    let mid = (band.s0 + band.s1) / 2.0;
    let mut s0 = band.s0 + 0.06;
    let mut s1 = band.s1 - 0.06;
    for o in f.obstacles.iter().chain(&f.placed) {
        if o.w1 > w && o.w0 < w + FACE_PANEL_M + 0.004 && o.y1 > y0 && o.y0 < y1 && o.s1 > s0 && o.s0 < s1 {
            if (o.s0 + o.s1) / 2.0 < mid {
                s0 = s0.max(o.s1 + 0.03);
            } else {
                s1 = s1.min(o.s0 - 0.03);
            }
        }
    }
    s0 += inset;
    s1 -= inset;
    if s1 - s0 < 0.9 {
        return None;
    }
    let shapes = sign_design(r, s1 - s0, y1 - y0, field);
    let b = Bx::new(s0, s1, y0, y1, w, w + FACE_PANEL_M);
    if !f.free(&b) {
        return None;
    }
    Some(SignFace { face: fi, s0, s1, y0, y1, w, field, shapes, glow: if lit { glow_code(false, key) } else { 0 }, letters: None })
}

fn blade_box(bl: &Blade) -> Bx {
    Bx::new(bl.s - 0.09, bl.s + 0.09, bl.y0 - 0.12, bl.y1 + 0.12, 0.0, bl.w1 + 0.06)
}

fn fit_blade(f: &Face, fi: usize, s0: f64, s1: f64, plan: &crate::elevation::Plan, r: &mut Rng, key: u32) -> Option<Blade> {
    let base = plan.base_y;
    let st1 = plan.storeys.get(1)?;
    let tall = r.next() < 0.18;
    let h = if tall { 1.8 + r.next() * 1.2 } else { 0.7 + r.next() * 0.6 };
    let proj = (0.5 + r.next() * 0.4).min(BLADE_MAX_PROJECTION_M).min(f.reach - 0.2);
    let neon = r.next() < 0.3;
    let field = if neon { [28, 30, 34] } else { pick(&SIGN_FIELDS, r.next()) };
    let tubes = pick(&NEON, r.next());
    let lit = neon || r.next() < 0.75;
    if proj < 0.4 {
        return None;
    }
    let y0 = (base + st1.y0 + 0.25).max(base + plan.storeys[0].y0 + UNDER_CLEAR_M + 0.3);
    let y1 = (y0 + h).min(base + plan.envelope.wall_top - 0.4);
    if y1 - y0 < 0.5 {
        return None;
    }
    let shapes = sign_design(r, y1 - y0, proj, field); // laid along the board (turned upright at draw)
    let w0 = 0.14;
    // on a wall pier between the windows of the storeys it spans.
    let spans = f.wall_spans(y0 - 0.12, y1 + 0.12, 0.06);
    for s in [s1 - 0.12, s0 + 0.12, (s0 + s1) / 2.0] {
        if !spans.iter().any(|(a, b)| s - 0.1 >= *a && s + 0.1 <= *b) {
            continue;
        }
        let bl = Blade { face: fi, s, y0, y1, w0, w1: w0 + proj, neon, field, shapes: shapes.clone(), tubes, glow: if lit { glow_code(neon, key) } else { 0 }, letters: None };
        if f.free(&blade_box(&bl)) {
            return Some(bl);
        }
    }
    None
}

fn awning_box(aw: &Awning) -> Bx {
    Bx::new(aw.s0, aw.s1, aw.top - aw.fall - aw.valance - 0.02, aw.top + 0.06, 0.0, aw.proj + 0.02)
}

#[allow(clippy::too_many_arguments)]
fn fit_awning(f: &Face, fi: usize, s0: f64, s1: f64, head: f64, floor: f64, ap: &crate::appearance::Appearance, r: &mut Rng) -> Option<Awning> {
    let form = weighted(&[(0u8, 0.45), (1, 0.15), (2, 0.2), (3, 0.2)], r.next());
    let proj = (0.85 + r.next() * 0.35).min(AWNING_MAX_PROJECTION_M).min(f.reach - 0.3);
    let valance = if form == 2 {
        0.3 + r.next() * 0.15
    } else if form == 3 {
        0.12
    } else {
        0.18 + r.next() * 0.1
    };
    let solid = pick(&AWNING_SOLIDS, r.next());
    let fade = (ap.paint * 0.5 + r.next() * 0.25).min(0.55) * if r.next() < 0.7 { 1.0 } else { 0.3 };
    let striped = r.next() < 0.22;
    let stripe_w = 0.12 + r.next() * 0.1;
    let torn_roll = r.next();
    let torn_at = r.next();
    let torn_w = 0.25 + r.next() * 0.5;
    let closed = r.next() < 0.7;
    // the top: under the fascia board and its mould over the shop, else over its head.
    let band = f.obstacles.iter().filter(|o| o.s1 > s0 && o.s0 < s1 && o.y0 >= head - 0.05 && o.y0 < head + 0.8 && o.w1 < 0.2).map(|o| o.y0).fold(f64::INFINITY, f64::min);
    let top = if band.is_finite() { band - 0.06 } else { head + 0.22 };
    let want_fall = match form {
        1 => proj * 0.8,
        2 => proj * 0.35,
        3 => proj * 0.27,
        _ => proj * 0.55,
    };
    let fall = want_fall.min(top - valance - (floor + UNDER_CLEAR_M));
    if fall < 0.18 || proj < 0.6 {
        return None;
    }
    // between the piers standing out over the awning's band.
    let (mut a, mut b) = (s0 - 0.08, s1 + 0.08);
    let mid = (s0 + s1) / 2.0;
    for o in f.obstacles.iter().chain(&f.placed) {
        if o.w1 > 0.03 && o.y1 > top - fall - valance && o.y0 < top + 0.06 && o.s1 > a && o.s0 < b {
            if (o.s0 + o.s1) / 2.0 < mid {
                a = a.max(o.s1 + 0.02);
            } else {
                b = b.min(o.s0 - 0.02);
            }
        }
    }
    a = a.max(0.05);
    b = b.min(f.edge.length - 0.05);
    if b - a < 1.0 {
        return None;
    }
    let colour = mix(solid, BLEACH, fade * 0.6);
    let stripe = striped.then(|| (mix(STRIPE_WHITE, BLEACH, fade * 0.3), stripe_w));
    let torn = (torn_roll < 0.12 * (0.5 + ap.wear)).then(|| (a + (b - a) * (0.2 + torn_at * 0.6), torn_w));
    let aw = Awning { face: fi, s0: a, s1: b, top, proj, fall, valance, form, colour, stripe, torn, closed_sides: form != 3 && closed };
    f.free(&awning_box(&aw)).then_some(aw)
}

fn shutter_boxes(sh: &Shutter) -> Vec<Bx> {
    vec![
        Bx::new(sh.s0 - 0.07, sh.s1 + 0.07, sh.head, sh.head + 0.32, sh.w0, sh.w0 + 0.28),
        Bx::new(sh.s0 - 0.07, sh.s0, sh.floor, sh.head, sh.w0, sh.w0 + 0.09),
        Bx::new(sh.s1, sh.s1 + 0.07, sh.floor, sh.head, sh.w0, sh.w0 + 0.09),
        Bx::new(sh.s0, sh.s1, sh.floor + 0.01, sh.head, sh.w0 + 0.03, sh.w0 + 0.085),
    ]
}

#[allow(clippy::too_many_arguments)]
fn fit_shutter(f: &Face, fi: usize, s0: f64, s1: f64, floor: f64, head: f64, down: bool, colour: [u8; 3]) -> Option<Shutter> {
    // the hood mounts over the head, in front of whatever board or pier is there.
    let w0 = f.standout(s0 - 0.08, s1 + 0.08, floor + 0.02, head + 0.34) + 0.004;
    if w0 > 0.2 || w0 + 0.3 > f.reach || s1 - s0 < 0.9 {
        return None;
    }
    let sh = Shutter { face: fi, s0, s1, floor, head, w0, down, colour };
    shutter_boxes(&sh).iter().all(|b| f.free(b)).then_some(sh)
}

#[allow(clippy::too_many_arguments)]
fn fit_grille(f: &Face, fi: usize, s0: f64, s1: f64, y0: f64, y1: f64, colour: [u8; 3]) -> Option<Grille> {
    // over the opening and its frame, in front of its sill and surround.
    let (a, b, c, d) = (s0 - 0.06, s1 + 0.06, y0 - 0.03, y1 + 0.05);
    let w0 = f.standout(a, b, c, d) + 0.004;
    if w0 > 0.16 {
        return None;
    }
    let g = Grille { face: fi, s0: a, s1: b, y0: c, y1: d, w0, colour };
    f.free(&Bx::new(a, b, c, d, w0, w0 + 0.026)).then_some(g)
}

fn fit_boarding(f: &Face, fi: usize, s0: f64, s1: f64, floor: f64, head: f64) -> Option<Boarding> {
    let w0 = f.standout(s0, s1, floor + 0.05, head) + 0.004;
    if w0 > 0.15 {
        return None;
    }
    let bd = Boarding { face: fi, s0, s1, y0: floor + 0.02, y1: head, w0 };
    f.free(&Bx::new(s0, s1, bd.y0, bd.y1, w0, w0 + 0.02)).then_some(bd)
}

fn downspout_boxes(ds: &Downspout) -> Vec<Bx> {
    let (a, b) = (ds.s - DS_W_M / 2.0 - 0.012, ds.s + DS_W_M / 2.0 + 0.012);
    let mut v: Vec<Bx> = ds.runs.iter().map(|(y0, y1)| Bx::new(a, b, *y0, *y1, 0.0, ds.w0 + DS_D_M + 0.006)).collect();
    if let Some(h) = ds.head {
        v.push(Bx::new(ds.s - 0.15, ds.s + 0.15, h, h + 0.3, 0.0, ds.w0 + 0.2));
    }
    v
}

fn fit_downspout(f: &Face, fi: usize, at_end: bool, floor: f64, plan: &crate::elevation::Plan, paint: Paint) -> Option<Downspout> {
    let len = f.edge.length;
    let base = plan.base_y;
    let wall_top = base + plan.envelope.wall_top;
    // a wall strip clear of the openings (and their sills) on every storey, near the end.
    let mut s = None;
    for k in 0..8 {
        let d = 0.16 + k as f64 * 0.12;
        let t = if at_end { len - d } else { d };
        if t < 0.1 || t > len - 0.1 {
            continue;
        }
        if f.apertures.iter().all(|o| t + DS_W_M / 2.0 + 0.1 < o[0] || t - DS_W_M / 2.0 - 0.1 > o[1]) {
            s = Some(t);
            break;
        }
    }
    let s = s?;
    let (a, b) = (s - DS_W_M / 2.0 - 0.012, s + DS_W_M / 2.0 + 0.012);
    let near = |o: &&Bx| o.s1 > a && o.s0 < b && o.y1 > floor && o.y0 < wall_top + 1.0;
    // anything standing out more than a band (a bay, a balcony, a sign, an escape) stops it.
    if f.obstacles.iter().chain(&f.placed).filter(near).any(|o| o.w1 > 0.35 && o.y1 > floor + 0.2 && o.y0 < wall_top - 0.6) {
        return None;
    }
    // it stands in front of the thin bands (plinths, string courses) and passes
    // behind (is cut by) the thicker ones: the shop cornice, the fascia, the cornice.
    let thin = f.obstacles.iter().chain(&f.placed).filter(near).filter(|o| o.w1 <= 0.09).map(|o| o.w1).fold(0.0, f64::max);
    let w0 = (thin + 0.006).max(DS_STANDOFF_M);
    let mut cuts: Vec<(f64, f64)> = f.obstacles.iter().chain(&f.placed).filter(near).filter(|o| o.w1 > w0 && o.w0 < w0 + DS_D_M).map(|o| (o.y0, o.y1)).collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    // the top: under the cornice (the lowest cut reaching the roofline), else under the wall top.
    let top_cut = cuts.iter().filter(|c| c.1 >= wall_top - 0.8).map(|c| c.0).fold(f64::INFINITY, f64::min);
    let top = if top_cut.is_finite() { top_cut - 0.02 } else { wall_top - 0.15 };
    let head_y = top - 0.32;
    let mut runs = vec![];
    let mut at = floor + 0.02;
    for (y0, y1) in cuts.iter().filter(|c| c.0 < head_y && c.1 > floor + 0.02) {
        if *y0 - 0.005 > at + 0.1 {
            runs.push((at, *y0 - 0.005));
        }
        at = at.max(*y1 + 0.005);
    }
    if head_y > at + 0.1 {
        runs.push((at, head_y));
    }
    if runs.iter().map(|r| r.1 - r.0).sum::<f64>() < 3.0 {
        return None;
    }
    // straps every 1.8 m, clear of anything standing out from the wall at their height.
    let mut straps = vec![];
    for (y0, y1) in &runs {
        let mut y = y0 + 0.6;
        while y < y1 - 0.3 {
            if f.standout(a, b, y - 0.01, y + 0.04) < 0.006 {
                straps.push(y);
            }
            y += 1.8;
        }
    }
    let mut ds = Downspout { face: fi, s, w0, runs, straps, head: Some(head_y), paint };
    let pieces = downspout_boxes(&ds);
    let (head, runs) = pieces.split_last().unwrap();
    if !runs.iter().all(|r| f.free(r)) {
        return None;
    }
    if !f.free(head) {
        ds.head = None;
    }
    Some(ds)
}

fn meter_boxes(m: &Meters) -> Vec<Bx> {
    let w = 0.3 * m.electric as f64 + if m.gas { 0.5 } else { 0.0 };
    let mut v = vec![Bx::new(m.s - 0.02, m.s + w + 0.02, m.floor - 0.05, m.conduit_top + 0.06, m.w0, m.w0 + 0.3)];
    if let Some((fs, _)) = m.fdc {
        v.push(Bx::new(fs - 0.27, fs + 0.27, m.floor + 0.62, m.floor + 0.98, 0.0, 0.3));
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn fit_meters(f: &Face, fi: usize, floor: f64, electric: u32, gas: bool, fdc: bool, r: &mut Rng, l: &Limits, paint: Paint) -> Option<Meters> {
    let width = 0.3 * electric as f64 + if gas { 0.5 } else { 0.0 } + 0.1;
    let conduit_top = floor + 2.4 + r.next() * 1.0;
    let side = r.next() < 0.5;
    let fdc_colour = if r.next() < 0.6 { BRASS } else { FDC_RED };
    let spans = f.wall_spans(floor, conduit_top, 0.06);
    let mut cands: Vec<(f64, f64)> = spans.into_iter().filter(|(a, b)| b - a >= width + 0.1).collect();
    if cands.is_empty() {
        return None;
    }
    // nearer an end first.
    let len = f.edge.length;
    cands.sort_by(|a, b| a.0.min(len - a.1).total_cmp(&b.0.min(len - b.1)));
    for (a, b) in cands.into_iter().take(3) {
        let s = if side { a + 0.05 } else { b - width + 0.05 };
        let w0 = f.standout(s - 0.05, s + width, floor, conduit_top) + 0.004;
        if w0 > 0.2 || w0 + 0.3 > l.frontage_m.min(f.reach) {
            continue;
        }
        let mut m = Meters { face: fi, s, floor, w0, electric, gas, conduit_top, fdc: None, paint };
        if !meter_boxes(&m).iter().all(|b| f.free(b)) {
            continue;
        }
        if fdc {
            // the standpipe connection beside the entrance, on flat wall.
            if let Some(o) = f.openings.iter().find(|o| o.storey == 0 && o.type_ == "entrance") {
                for fs in [o.s1 + 0.4, o.s0 - 0.4] {
                    m.fdc = Some((fs, fdc_colour));
                    let fb = *meter_boxes(&m).last().unwrap();
                    let flat = f.standout(fb.s0, fb.s1, fb.y0, fb.y1) < 0.02;
                    let clear = f.apertures.iter().all(|q| fb.s1 < q[0] || fb.s0 > q[1] || fb.y1 < q[2] || fb.y0 > q[3]);
                    if flat && clear && f.free(&fb) && !fb.hits(&meter_boxes(&m)[0], 0.0) {
                        break;
                    }
                    m.fdc = None;
                }
            }
        }
        return Some(m);
    }
    None
}

fn fit_scaffold(f: &Face, fi: usize, floor: f64, wall_top: f64) -> Option<Scaffold> {
    let len = f.edge.length;
    let (s0, s1) = (0.2, len - 0.2);
    let top = wall_top + 1.0;
    let w0 = f.standout(s0, s1, floor, top) + 0.12;
    if w0 > 0.5 || w0 + 1.15 > f.reach {
        return None;
    }
    let sc = Scaffold { face: fi, s0, s1, floor, top, w0 };
    f.free(&Bx::new(s0, s1, floor, top, w0, w0 + 1.1)).then_some(sc)
}

/// Where a dish looks: azimuth 160 degrees and 45 degrees up (the
/// geostationary arc's broadcast satellites seen from 37.8 N, 122.4 W).
/// World +z is south, +x east.
fn dish_aim() -> V3 {
    let az = 160f64.to_radians();
    let el = 45f64.to_radians();
    norm([az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()])
}

fn plan_roof(planned: &Planned, ap: &crate::appearance::Appearance, year: f64, multi: bool, district: &str, out: &mut ClutterPlan) {
    let plan = &planned.plan;
    let frame = &planned.frame;
    let Some(slab) = plan.parts.iter().find(|p| p.kind == "roof-slab") else { return };
    let deck = slab.y1;
    let cap = plan.base_y + plan.envelope.roof_top + plan.envelope.roof_plant_allowance;
    let mut rng = Rng::new(&format!("{}|facade-clutter|roof", planned.classified.seed));
    // what stands on the roof already (world xz boxes).
    let mut taken: Vec<([f64; 2], [f64; 2])> = vec![];
    for p in &plan.parts {
        if p.edge.is_some() || p.kind == "roof-slab" || p.y1 < deck + 0.02 {
            continue;
        }
        let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
        for c in part_corners(p) {
            a = [a[0].min(c[0]), a[1].min(c[2])];
            b = [b[0].max(c[0]), b[1].max(c[2])];
        }
        taken.push((a, b));
    }
    let inset = 0.7;
    let u = [frame.right.x, frame.right.z];
    let footprint = |c: [f64; 2], h: [f64; 2]| -> ([f64; 2], [f64; 2], bool) {
        let v = [-u[1], u[0]];
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(a, b)| [c[0] + u[0] * h[0] * a + v[0] * h[1] * b, c[1] + u[1] * h[0] * a + v[1] * h[1] * b]);
        let inside = corners.iter().all(|p| frame.signed_distance(p[0], p[1]) < -inset);
        let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &corners {
            a = [a[0].min(p[0]), a[1].min(p[1])];
            b = [b[0].max(p[0]), b[1].max(p[1])];
        }
        (a, b, inside)
    };
    let fits = |c: [f64; 2], h: [f64; 2], taken: &Vec<([f64; 2], [f64; 2])>| -> bool {
        let (a, b, inside) = footprint(c, h);
        inside && !taken.iter().any(|t| a[0] < t.1[0] + 0.3 && t.0[0] < b[0] + 0.3 && a[1] < t.1[1] + 0.3 && t.0[1] < b[1] + 0.3)
    };
    let put = |c: [f64; 2], h: [f64; 2], y1: f64, kind: &'static str, taken: &mut Vec<([f64; 2], [f64; 2])>, out: &mut ClutterPlan| {
        let (a, b, _) = footprint(c, h);
        taken.push((a, b));
        out.roof_boxes.push((kind, [a[0], deck, a[1]], [b[0], y1, b[1]]));
    };
    let fw = frame.width / 2.0 - inset - 0.5;
    let fd = frame.depth / 2.0 - inset - 0.5;
    if fw < 0.6 || fd < 0.6 {
        return;
    }
    let to_world = |px: f64, pz: f64| {
        let p = frame.to_world(px, pz);
        [p.x, p.z]
    };
    // chimneys at the party walls of older buildings.
    let masonry = plan.materials.get("wall").map(|w| w.contains("brick") || w.contains("stone") || w.contains("stucco") || w.contains("plaster")).unwrap_or(false);
    if year < 1940.0 && (masonry || multi) {
        let n = if multi { 1 + (rng.next() * 2.4) as u32 } else { (rng.next() * 1.6) as u32 };
        for _ in 0..n {
            let side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            let px = side * fw * (0.85 + rng.next() * 0.15);
            let pz = (rng.next() - 0.5) * 2.0 * fd * 0.8;
            let half = [0.22 + rng.next() * 0.18, 0.3 + rng.next() * 0.2];
            let h = 1.0 + rng.next() * 1.2;
            let pots = if rng.next() < 0.55 { 1 + (rng.next() * 2.5) as u32 } else { 0 };
            let c = to_world(px, pz);
            let y1 = (deck + h).min(cap - 0.5);
            if y1 - deck > 0.6 && fits(c, [half[0] + 0.05, half[1] + 0.05], &taken) {
                put(c, [half[0] + 0.05, half[1] + 0.05], y1 + 0.5, "chimney", &mut taken, out);
                out.roof.push(Roof::Chimney { c, u, half, y0: deck - 0.02, y1, pots });
                out.census.chimneys += 1;
            }
        }
    }
    // vent stacks and flue caps (plumbing vents through every residential roof).
    let nv = if multi { 3 + (rng.next() * 5.0) as u32 } else { 1 + (rng.next() * 3.0) as u32 };
    for _ in 0..nv {
        let c = to_world((rng.next() - 0.5) * 2.0 * fw, (rng.next() - 0.5) * 2.0 * fd);
        let r = 0.035 + rng.next() * 0.03;
        let h = 0.3 + rng.next() * 0.55;
        let cap_on = rng.next() < 0.4;
        let dark = rng.next() < 0.4;
        if fits(c, [r * 2.2 + 0.02, r * 2.2 + 0.02], &taken) {
            let y1 = (deck + h).min(cap - 0.1);
            put(c, [r * 2.2 + 0.02, r * 2.2 + 0.02], y1 + 0.1, "vent", &mut taken, out);
            out.roof.push(Roof::Vent { c, r, y0: deck - 0.02, y1, cap: cap_on, dark });
            out.census.vents += 1;
        }
    }
    // satellite dishes on apartment roofs.
    let p_dish = if multi { 0.28 } else { 0.08 } * street_density(district);
    if rng.next() < p_dish {
        let nd = 1 + (rng.next() * 2.2) as u32;
        for _ in 0..nd {
            let c = to_world((rng.next() - 0.5) * 2.0 * fw, (rng.next() - 0.5) * 2.0 * fd);
            let r = 0.28 + rng.next() * 0.16;
            if fits(c, [r + 0.45, r + 0.45], &taken) && deck + 1.4 <= cap {
                put(c, [r + 0.45, r + 0.45], deck + 1.4, "dish", &mut taken, out);
                out.dishes.push(Dish { at: [c[0], deck + 0.8, c[1]], r, look: dish_aim(), mount: None });
                out.census.dishes += 1;
            }
        }
    }
    // a TV aerial on older homes and flats.
    let r_aerial = rng.next();
    let (ax, az) = ((rng.next() - 0.5) * fw, (rng.next() - 0.5) * fd);
    let h = 2.2 + rng.next() * 1.4;
    let yaw = rng.next() * std::f64::consts::PI;
    let elements = 5 + (rng.next() * 5.0) as u32;
    if year < 1965.0 && r_aerial < 0.07 + ap.wear * 0.06 {
        let c = to_world(ax, az);
        if fits(c, [0.95, 0.95], &taken) && deck + h + 0.1 <= cap {
            put(c, [0.95, 0.95], deck + h + 0.1, "aerial", &mut taken, out);
            out.roof.push(Roof::Aerial { c, y0: deck - 0.02, h, yaw, elements });
            out.census.aerials += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing (per rung).
// ---------------------------------------------------------------------------

/// The finish and paint a service fitting draws with.
fn paint_of(p: Paint, drawn: &DrawnMaterials) -> (String, u32) {
    match p {
        Paint::Wall => {
            let wall = drawn.materials.get("wall").cloned().unwrap_or_else(|| M_GALV.to_string());
            let t = drawn.tints.get(&wall).copied().unwrap_or(0xffffff);
            (wall, t)
        }
        Paint::Galv => (M_GALV.to_string(), tint(M_GALV, GALV)),
        Paint::Dark => (M_STEEL.to_string(), tint(M_STEEL, [44, 46, 46])),
        Paint::Colour(c) => {
            let m = finish_for(c, M_GALV);
            (m.to_string(), tint(m, c))
        }
    }
}

/// Draw `cp` for rung `rung` (0 near: all of it; 2 mid: simplified) into `out`.
pub fn emit(cp: &ClutterPlan, rung: u32, drawn: &DrawnMaterials, out: &mut RungMesh) {
    let near = rung == 0;
    let mut m = Mesher::default();
    let base_of = |fi: usize| cp.bases[fi].1;
    for esc in &cp.escapes {
        draw_escape(&mut m, &base_of(esc.face), esc, near);
    }
    for aw in &cp.awnings {
        draw_awning(&mut m, &base_of(aw.face), aw, near);
    }
    for sf in &cp.faces_signs {
        draw_sign_face(&mut m, &base_of(sf.face), sf, near);
    }
    for bl in &cp.blades {
        draw_blade(&mut m, &base_of(bl.face), bl, near);
    }
    for ws in &cp.window_signs {
        draw_window_sign(&mut m, &base_of(ws.face), ws);
    }
    for sh in &cp.shutters {
        draw_shutter(&mut m, &base_of(sh.face), sh, near);
    }
    for bd in &cp.boards {
        draw_boarding(&mut m, &base_of(bd.face), bd, near);
    }
    if near {
        for g in &cp.grilles {
            draw_grille(&mut m, &base_of(g.face), g);
        }
    }
    for ds in &cp.downspouts {
        let (mat, paint) = paint_of(ds.paint, drawn);
        draw_downspout(&mut m, &base_of(ds.face), ds, &mat, paint, near);
    }
    if near {
        for mt in &cp.meters {
            draw_meters(&mut m, &base_of(mt.face), mt, drawn);
        }
        for ac in &cp.acs {
            draw_ac(&mut m, &base_of(ac.face), ac);
        }
    }
    for d in &cp.dishes {
        draw_dish(&mut m, d, near);
    }
    for sc in &cp.scaffolds {
        draw_scaffold(&mut m, &base_of(sc.face), sc, near);
    }
    for r in &cp.roof {
        draw_roof(&mut m, r, near);
    }
    if m.groups.is_empty() {
        return;
    }
    if out.glow.len() < out.groups.len() {
        out.glow.resize(out.groups.len(), 0);
    }
    // (stage S1) the lettered groups' vertex bytes, one list a group (empty for a plain one).
    let lettered = m.codes.iter().any(|c| c.is_some());
    if lettered && out.letters.len() < out.groups.len() {
        out.letters.resize(out.groups.len(), vec![]);
    }
    for ((mat, paint, glow, sink), codes) in m.groups.into_iter().zip(m.codes) {
        if sink.indices.is_empty() {
            continue;
        }
        out.groups.push((mat, Some(paint), sink));
        out.glow.push(glow);
        if lettered {
            out.letters.push(codes.unwrap_or_default());
        }
    }
}

fn steel_finish(c: [u8; 3]) -> (&'static str, u32) {
    let m = finish_for(c, M_STEEL);
    (m, tint(m, c))
}

fn draw_escape(m: &mut Mesher, e: &Eb, esc: &Escape, near: bool) {
    let paint = steel_finish(esc.paint);
    let rust_c = [steel_finish(RUST[0]), steel_finish(RUST[1])];
    let mut rr = Rng::from_hash(esc.seed ^ 0x2545_f491);
    let (s0, s1) = (esc.s0, esc.s1);
    let (w0, w1) = (esc.w0, esc.w0 + esc.depth);
    // a piece rusts by the building's wear (`k`: how exposed that kind of piece is).
    let mut fin = |k: f64| -> (&'static str, u32) {
        let a = rr.next();
        let b = rr.next();
        if a < esc.rust * k { rust_c[(b * 2.0) as usize % 2] } else { paint }
    };
    let (sw0, sw1) = (w1 - FE_STAIR_W_M - 0.05, w1 - 0.04);
    let n_decks = esc.decks.len();
    for (k, &d) in esc.decks.iter().enumerate() {
        // the hatch over the top of the flight that arrives from below.
        let hatch = (k > 0).then(|| {
            let (_, head) = flight(esc, k - 1);
            let run = (d - esc.decks[k - 1]) * FE_STAIR_RUN_PER_RISE;
            let hl = run.min(1.1);
            if esc.flights_from_s0[k - 1] { (head - hl, head) } else { (head, head + hl) }
        });
        // the frame: channels round the deck.
        {
            let sk = m.at(paint.0, paint.1, 0);
            ebox(sk, e, s0, s1, d - 0.075, d, w1 - 0.04, w1, F_ALL);
            ebox(sk, e, s0, s1, d - 0.075, d, w0, w0 + 0.04, F_ALL);
            ebox(sk, e, s0, s0 + 0.04, d - 0.075, d, w0 + 0.04, w1 - 0.04, F_ALONG_W | F_S0);
            ebox(sk, e, s1 - 0.04, s1, d - 0.075, d, w0 + 0.04, w1 - 0.04, F_ALONG_W | F_S1);
        }
        if near {
            // slats (flat bar on edge, along the wall), the hatch left open.
            let mut w = w0 + 0.05;
            while w + FE_SLAT_W_M <= w1 - 0.045 {
                let f = fin(0.4);
                let sk = m.at(f.0, f.1, 0);
                let in_stair = w + FE_SLAT_W_M > sw0 && w < sw1;
                match (hatch, in_stair) {
                    (Some((a, b)), true) => {
                        if a > s0 + 0.08 {
                            ebox(sk, e, s0 + 0.04, a, d - 0.03, d - 0.004, w, w + FE_SLAT_W_M, F_ALONG_S);
                        }
                        if b < s1 - 0.08 {
                            ebox(sk, e, b, s1 - 0.04, d - 0.03, d - 0.004, w, w + FE_SLAT_W_M, F_ALONG_S);
                        }
                    }
                    _ => ebox(sk, e, s0 + 0.04, s1 - 0.04, d - 0.03, d - 0.004, w, w + FE_SLAT_W_M, F_ALONG_S),
                }
                w += FE_SLAT_PITCH_M;
            }
            // cross bearers under the slats.
            let sk = m.at(paint.0, paint.1, 0);
            let nb = (((s1 - s0) / 1.1).ceil() as usize).max(2);
            for i in 1..nb {
                let s = s0 + (s1 - s0) * i as f64 / nb as f64;
                ebox(sk, e, s - 0.02, s + 0.02, d - 0.075, d - 0.03, w0 + 0.04, w1 - 0.04, F_ALONG_W);
            }
        } else {
            // the mid rung: the deck as one slab.
            let sk = m.at(paint.0, paint.1, 0);
            ebox(sk, e, s0 + 0.04, s1 - 0.04, d - 0.03, d - 0.004, w0 + 0.04, w1 - 0.04, F_TOP | F_BOT);
        }
        // the railing: top rail round the front and both ends, a kick rail, corner posts.
        let top = d + FE_RAIL_M;
        {
            let sk = m.at(paint.0, paint.1, 0);
            let rh = 0.02;
            ebox(sk, e, s0, s1, top - rh * 2.0, top, w1 - rh * 2.0, w1, F_ALONG_S);
            ebox(sk, e, s0, s0 + rh * 2.0, top - rh * 2.0, top, w0, w1 - rh * 2.0, F_ALONG_W);
            ebox(sk, e, s1 - rh * 2.0, s1, top - rh * 2.0, top, w0, w1 - rh * 2.0, F_ALONG_W);
            let kick = d + 0.12;
            ebox(sk, e, s0, s1, kick - 0.02, kick, w1 - 0.025, w1 - 0.005, F_ALONG_S);
            for (ps, pw) in [(s0, w1 - 0.04), (s1 - 0.04, w1 - 0.04), (s0, w0), (s1 - 0.04, w0)] {
                ebox(sk, e, ps, ps + 0.04, d, top - rh * 2.0, pw, pw + 0.04, F_ALONG_Y);
            }
            if !near {
                // the mid rung: a middle rail for the pickets.
                ebox(sk, e, s0, s1, d + FE_RAIL_M * 0.5 - 0.02, d + FE_RAIL_M * 0.5, w1 - 0.025, w1 - 0.005, F_ALONG_S);
            }
        }
        if near {
            // pickets along the front and both ends.
            let mut s = s0 + 0.04 + FE_PICKET_PITCH_M;
            while s < s1 - 0.06 {
                let f = fin(0.15);
                let sk = m.at(f.0, f.1, 0);
                ebox(sk, e, s, s + FE_PICKET_M, d - 0.004, top - 0.04, w1 - 0.022, w1 - 0.006, F_ALONG_Y);
                s += FE_PICKET_PITCH_M;
            }
            for es in [s0 + 0.012, s1 - 0.012 - FE_PICKET_M] {
                let mut w = w0 + 0.04 + FE_PICKET_PITCH_M;
                while w < w1 - 0.06 {
                    let f = fin(0.15);
                    let sk = m.at(f.0, f.1, 0);
                    ebox(sk, e, es, es + FE_PICKET_M, d - 0.004, top - 0.04, w, w + FE_PICKET_M, F_ALONG_Y);
                    w += FE_PICKET_PITCH_M;
                }
            }
        }
        // knee brackets from the wall up to the deck's outer edge, at both ends.
        {
            let f = fin(0.6);
            let sk = m.at(f.0, f.1, 0);
            let kw = esc.knee_w.get(k).copied().unwrap_or(0.0);
            for bs in [s0 + 0.08, s1 - 0.08] {
                let a = e.p(bs, d - 0.6, kw + 0.012);
                let b = e.p(bs, d - 0.078, w1 - 0.1);
                bar(sk, a, b, 0.012, 0.03, e.du(), false);
                if near {
                    // the plate bolted to the wall (or over the band there).
                    ebox(sk, e, bs - 0.04, bs + 0.04, d - 0.62, d - 0.5, kw, kw + 0.012, F_NO_BACK);
                }
            }
        }
        // the flight up to the next deck: two stringers, treads, a handrail.
        if k + 1 < n_decks {
            let d1 = esc.decks[k + 1];
            let (foot, head) = flight(esc, k);
            let f = fin(0.35);
            let sk = m.at(f.0, f.1, 0);
            if near {
                for sw in [sw0 + 0.012, sw1 - 0.012] {
                    bar(sk, e.p(foot, d - 0.07, sw), e.p(head, d1 - 0.12, sw), 0.075, 0.01, e.dn(), false);
                }
                let steps = ((d1 - d) / FE_TREAD_RISE_M).round().max(4.0) as usize;
                for i in 1..steps {
                    let t = i as f64 / steps as f64;
                    let s = foot + (head - foot) * t;
                    let y = d + (d1 - d) * t;
                    ebox(sk, e, s - 0.06, s + 0.06, y - 0.025, y, sw0 + 0.024, sw1 - 0.024, F_ALONG_W);
                }
                // the handrail on the wall side, on its two posts.
                let a = e.p(foot, d + 0.86, sw0 + 0.012);
                let b = e.p(head, d1 + 0.86, sw0 + 0.012);
                rod(sk, a, b, 0.012);
                rod(sk, e.p(foot, d, sw0 + 0.012), a, 0.012);
                rod(sk, e.p(head, d1, sw0 + 0.012), b, 0.012);
            } else {
                // the mid rung: the flight as a slab.
                let mw = (sw0 + sw1) / 2.0;
                bar(sk, e.p(foot, d - 0.03, mw), e.p(head, d1 - 0.06, mw), 0.04, (sw1 - sw0) / 2.0, e.dn(), false);
            }
        }
    }
    if let (Some((s, foot)), true) = (esc.ladder, near) {
        // the raised drop ladder outside the lowest rail: stiles hooked over it, rungs.
        let d0 = esc.decks[0];
        let f = fin(0.6);
        let sk = m.at(f.0, f.1, 0);
        let wl = w1 + 0.04;
        for ds in [-FE_LADDER_W_M / 2.0, FE_LADDER_W_M / 2.0] {
            ebox(sk, e, s + ds - 0.012, s + ds + 0.012, foot, d0 + FE_RAIL_M - 0.06, wl - 0.025, wl + 0.025, F_ALONG_Y | F_BOT);
            // the hook over the top rail.
            ebox(sk, e, s + ds - 0.012, s + ds + 0.012, d0 + FE_RAIL_M - 0.06, d0 + FE_RAIL_M + 0.004, w1 - 0.002, wl + 0.025, F_ALL);
        }
        let mut y = foot + 0.15;
        while y < d0 + FE_RAIL_M - 0.25 {
            rod(sk, e.p(s - FE_LADDER_W_M / 2.0, y, wl), e.p(s + FE_LADDER_W_M / 2.0, y, wl), 0.01);
            y += 0.3;
        }
    }
    if let (Some((s, top)), true) = (esc.roof_ladder, near) {
        // the gooseneck ladder from the top deck over the parapet.
        let d = *esc.decks.last().unwrap();
        let sk = m.at(paint.0, paint.1, 0);
        let wl = w1 - 0.3;
        let bend = top - 0.9;
        for ds in [-0.22, 0.22] {
            let a = e.p(s + ds, d, wl);
            let b = e.p(s + ds, bend + 0.3, wl);
            rod(sk, a, b, 0.014);
            let c = e.p(s + ds, top, wl - 0.3);
            rod(sk, b, c, 0.014);
            rod(sk, c, e.p(s + ds, top - 0.25, -0.15), 0.014);
        }
        let mut y = d + FE_RAIL_M + 0.3;
        while y < bend {
            rod(sk, e.p(s - 0.22, y, wl), e.p(s + 0.22, y, wl), 0.01);
            y += 0.3;
        }
    }
}

fn draw_awning(m: &mut Mesher, e: &Eb, aw: &Awning, near: bool) {
    let (top, p, fall, val) = (aw.top, aw.proj, aw.fall, aw.valance);
    let t = AW_FABRIC_M;
    // the fabric line in (w, y), wall to the front edge.
    let line: Vec<[f64; 2]> = match aw.form {
        1 => (0..=8)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / 8.0;
                [p * a.sin(), top - fall + fall * a.cos()]
            })
            .collect(),
        2 => vec![[0.0, top], [p, top - fall]],
        3 => vec![[0.0, top], [p, top - fall]],
        _ => vec![[0.0, top], [p * 0.5, top - fall * 0.52], [p, top - fall]],
    };
    // the fabric as a thin shell: the line, the valance down from the front,
    // back up its inside and along the line's underside (a simple polygon).
    let under = |q: &[f64; 2]| [q[0] - t * q[0] / p, q[1] - t];
    let front_y = top - fall;
    let drop = if aw.form == 1 { val * 0.6 } else { val };
    let mut prof: Vec<[f64; 2]> = line.clone();
    prof.push([p, front_y - drop]);
    prof.push([p - t, front_y - drop]);
    prof.extend(line.iter().rev().map(under));
    // stripes along s (each its own colour), or one run; a tear cuts the valance.
    let mut cuts: Vec<(f64, f64, [u8; 3])> = vec![];
    match aw.stripe {
        Some((white, wdt)) if near => {
            let mut s = aw.s0;
            let mut k = 0;
            while s < aw.s1 - 1e-6 {
                let b = (s + wdt).min(aw.s1);
                cuts.push((s, b, if k % 2 == 0 { aw.colour } else { white }));
                s = b;
                k += 1;
            }
        }
        Some((white, _)) => cuts.push((aw.s0, aw.s1, mix(aw.colour, white, 0.45))),
        None => cuts.push((aw.s0, aw.s1, aw.colour)),
    }
    for (a, b, colour) in &cuts {
        let sk = m.col(M_CANVAS, *colour, 0);
        match aw.torn {
            Some((at, width)) if near && *b > at - width / 2.0 && *a < at + width / 2.0 => {
                // the canopy runs on; the valance hangs torn (shorter, the tear's edge sagging).
                let canopy: Vec<[f64; 2]> = line.iter().copied().chain(line.iter().rev().map(under)).collect();
                sweep(sk, e, &canopy, *a, *b, false);
                let ta = (at - width / 2.0).max(*a);
                let tb = (at + width / 2.0).min(*b);
                let v = |s0: f64, s1: f64, drop: f64, sk: &mut Sink| {
                    if s1 - s0 > 0.01 {
                        ebox(sk, e, s0, s1, front_y - drop, front_y, p - t, p, F_OUT | F_IN | F_BOT);
                    }
                };
                v(*a, ta, val, sk);
                v(tb, *b, val, sk);
                v(ta, tb, val * 0.3, sk);
            }
            _ => sweep(sk, e, &prof, *a, *b, false),
        }
    }
    // the end panels (closed sides) or open sides.
    let sk = m.col(M_CANVAS, shade(aw.colour, 0.92), 0);
    if aw.closed_sides {
        let mut side: Vec<[f64; 2]> = vec![[0.0, front_y - if aw.form == 1 { drop } else { 0.0 }]];
        side.extend(line.iter().copied());
        if aw.form == 1 {
            side.push([p, front_y - drop]);
        }
        for s in [aw.s0, aw.s1] {
            let pts: Vec<V3> = side.iter().map(|q| e.p(s, q[1], q[0])).collect();
            fan(sk, &pts, if s == aw.s0 { neg(e.du()) } else { e.du() });
        }
    }
    // the frame: a wall rail, the front bar, and (open retractable) the folding arms.
    let fr = steel_finish([52, 52, 50]);
    let sk = m.at(fr.0, fr.1, 0);
    ebox(sk, e, aw.s0, aw.s1, top - 0.03, top + 0.05, 0.0, 0.05, F_NO_BACK);
    if near {
        rod(sk, e.p(aw.s0, front_y - 0.02, p - 0.03), e.p(aw.s1, front_y - 0.02, p - 0.03), 0.018);
        if aw.form == 3 {
            for s in [aw.s0 + 0.15, aw.s1 - 0.15] {
                rod(sk, e.p(s, top - 0.35, 0.03), e.p(s, front_y - 0.04, p - 0.05), 0.016);
            }
        } else if !aw.closed_sides {
            for s in [aw.s0 + 0.02, aw.s1 - 0.02] {
                rod(sk, e.p(s, top - 0.02, 0.02), e.p(s, front_y - 0.02, p - 0.03), 0.012);
            }
        }
    }
}

/// A sign's shapes in a plane: (o, ax across, ay up, normal out); `t` how proud.
#[allow(clippy::too_many_arguments)]
fn draw_shapes(m: &mut Mesher, o: V3, ax: V3, ay: V3, shapes: &[Shape2], t0: f64, t1: f64, glow: u8, both: bool) {
    for sh in shapes {
        let [cx, cy] = sh.c;
        let [rx, ry] = sh.r;
        let sk = m.col(M_PANEL, sh.colour, glow);
        let poly = |pts: Vec<[f64; 2]>, sk: &mut Sink| plate(sk, o, ax, ay, &pts, t0, t1, both);
        match sh.kind {
            0 => poly((0..14).map(|i| { let a = std::f64::consts::TAU * i as f64 / 14.0; [cx + rx * a.cos(), cy + ry * a.sin()] }).collect(), sk),
            1 => {
                // a ring: segments between the outer and inner radius.
                let n = 16;
                for i in 0..n {
                    let a0 = std::f64::consts::TAU * i as f64 / n as f64;
                    let a1 = std::f64::consts::TAU * (i + 1) as f64 / n as f64;
                    let pts = vec![[cx + rx * a0.cos(), cy + rx * a0.sin()], [cx + rx * a1.cos(), cy + rx * a1.sin()], [cx + ry * a1.cos(), cy + ry * a1.sin()], [cx + ry * a0.cos(), cy + ry * a0.sin()]];
                    poly(pts, sk);
                }
            }
            2 => poly(vec![[cx, cy - ry], [cx + rx, cy], [cx, cy + ry], [cx - rx, cy]], sk),
            5 => poly((0..=10).map(|i| { let a = std::f64::consts::PI * i as f64 / 10.0; [cx + rx * a.cos(), cy + ry * a.sin()] }).collect(), sk),
            _ => poly(vec![[cx - rx, cy - ry], [cx + rx, cy - ry], [cx + rx, cy + ry], [cx - rx, cy + ry]], sk),
        }
    }
}

/// (stage S1) A lettered quad (corners in `ebox` order), its text box
/// centred at `c0` with axes `ax_u` (reading) and `ax_v` (up the letters),
/// `bw` x `bh` m: each corner carries its (u, v) in the box.
#[allow(clippy::too_many_arguments)]
fn letter_quad(m: &mut Mesher, glow: u8, code: LetterCode, pts: [V3; 4], n: V3, c0: V3, ax_u: V3, ax_v: V3, bw: f64, bh: f64) {
    let (sink, codes) = m.lettered(glow);
    quad(sink, pts[0], pts[1], pts[2], pts[3], n);
    for p in pts {
        let d = sub(p, c0);
        codes.push(code.bytes(dot3(d, ax_u) / bw + 0.5, dot3(d, ax_v) / bh + 0.5));
    }
}

/// (stage S1) A lettered sign's edge: painted, in its field, out of every text box.
fn letter_rim(m: &mut Mesher, glow: u8, code: LetterCode, pts: [V3; 4], n: V3) {
    let rim = LetterCode { mode: LETTER_PAINTED, emblem: EMBLEM_NONE, query: 0, ..code };
    let (sink, codes) = m.lettered(glow);
    quad(sink, pts[0], pts[1], pts[2], pts[3], n);
    codes.extend([rim.bytes(LETTER_UV_MIN, LETTER_UV_MIN); 4]);
}

/// (stage S1) The reader's right on a face whose outward normal is `n` (horizontal).
fn reader_right(n: V3) -> V3 {
    cross([0.0, 1.0, 0.0], n)
}

/// (stage S1) A lettered fascia panel: its front carries the words (and the
/// emblem or border), its edges are painted in the field.
fn draw_lettered_face(m: &mut Mesher, e: &Eb, sf: &SignFace, l: &Lettering) {
    let (s0, s1, y0, y1, w0, w1) = (sf.s0, sf.s1, sf.y0, sf.y1, sf.w, sf.w + FACE_PANEL_M);
    let p = |s: f64, y: f64, w: f64| e.p(s, y, w);
    let (u, n, up) = (e.du(), e.dn(), [0.0, 1.0, 0.0]);
    let c0 = p(l.centre[0], l.centre[1], w1);
    letter_quad(m, l.glow, l.code, [p(s0, y0, w1), p(s1, y0, w1), p(s1, y1, w1), p(s0, y1, w1)], n, c0, reader_right(n), up, l.aspect * l.height, l.height);
    letter_rim(m, l.glow, l.code, [p(s1, y0, w1), p(s1, y0, w0), p(s1, y1, w0), p(s1, y1, w1)], u);
    letter_rim(m, l.glow, l.code, [p(s0, y0, w0), p(s0, y0, w1), p(s0, y1, w1), p(s0, y1, w0)], neg(u));
    letter_rim(m, l.glow, l.code, [p(s0, y1, w1), p(s1, y1, w1), p(s1, y1, w0), p(s0, y1, w0)], up);
    letter_rim(m, l.glow, l.code, [p(s0, y0, w0), p(s1, y0, w0), p(s1, y0, w1), p(s0, y0, w1)], [0.0, -1.0, 0.0]);
}

/// (stage S1) A lettered blade board's two faces: the same words on each,
/// reading left to right from either side; `query` names the shop's front
/// (the wall's outward normal) as the reader's right or left.
fn draw_lettered_board(m: &mut Mesher, e: &Eb, bl: &Blade, l: &Lettering) {
    let th = BLADE_BOARD_M / 2.0;
    let (s, y0, y1, w0, w1) = (bl.s, bl.y0, bl.y1, bl.w0, bl.w1);
    let (c_w, c_y) = ((w0 + w1) / 2.0, (y0 + y1) / 2.0);
    let (u, dn, up) = (e.du(), e.dn(), [0.0, 1.0, 0.0]);
    let p = |s: f64, y: f64, w: f64| e.p(s, y, w);
    let (along, across) = (l.aspect * l.height, l.height);
    for (side, n) in [(1.0f64, u), (-1.0f64, neg(u))] {
        let sx = s + side * th;
        let pts = if side > 0.0 { [p(sx, y0, w1), p(sx, y0, w0), p(sx, y1, w0), p(sx, y1, w1)] } else { [p(sx, y0, w0), p(sx, y0, w1), p(sx, y1, w1), p(sx, y1, w0)] };
        let r = reader_right(n);
        let (ax_u, ax_v) = match l.layout {
            0 => (r, up),
            1 => (up, neg(r)),
            _ => ([0.0, -1.0, 0.0], neg(r)),
        };
        let code = LetterCode { query: if dot3(r, dn) > 0.0 { 1 } else { 2 }, ..l.code };
        letter_quad(m, l.glow, code, pts, n, p(sx, c_y, c_w), ax_u, ax_v, along, across);
    }
}

/// (stage S1) A window's neon sign: the words on its front, its edges dark.
fn draw_window_sign(m: &mut Mesher, e: &Eb, ws: &WindowSign) {
    let (s0, s1, y0, y1, w0, w1) = (ws.s0, ws.s1, ws.y0, ws.y1, ws.w0, ws.w0 + WINDOW_SIGN_M);
    let l = &ws.letters;
    let p = |s: f64, y: f64, w: f64| e.p(s, y, w);
    let (u, n, up) = (e.du(), e.dn(), [0.0, 1.0, 0.0]);
    letter_quad(m, l.glow, l.code, [p(s0, y0, w1), p(s1, y0, w1), p(s1, y1, w1), p(s0, y1, w1)], n, p(l.centre[0], l.centre[1], w1), reader_right(n), up, l.aspect * l.height, l.height);
    letter_rim(m, l.glow, l.code, [p(s1, y0, w1), p(s1, y0, w0), p(s1, y1, w0), p(s1, y1, w1)], u);
    letter_rim(m, l.glow, l.code, [p(s0, y0, w0), p(s0, y0, w1), p(s0, y1, w1), p(s0, y1, w0)], neg(u));
    letter_rim(m, l.glow, l.code, [p(s0, y1, w1), p(s1, y1, w1), p(s1, y1, w0), p(s0, y1, w0)], up);
    letter_rim(m, l.glow, l.code, [p(s0, y0, w0), p(s1, y0, w0), p(s1, y0, w1), p(s0, y0, w1)], [0.0, -1.0, 0.0]);
}

fn draw_sign_face(m: &mut Mesher, e: &Eb, sf: &SignFace, near: bool) {
    if let Some(l) = &sf.letters {
        draw_lettered_face(m, e, sf, l);
        return;
    }
    let sk = m.col(M_PANEL, sf.field, sf.glow);
    ebox(sk, e, sf.s0, sf.s1, sf.y0, sf.y1, sf.w, sf.w + FACE_PANEL_M, F_NO_BACK);
    if near {
        let c = e.p((sf.s0 + sf.s1) / 2.0, (sf.y0 + sf.y1) / 2.0, sf.w + FACE_PANEL_M);
        draw_shapes(m, c, e.du(), [0.0, 1.0, 0.0], &sf.shapes, 0.0, 0.004, sf.glow, false);
    }
}

fn draw_blade(m: &mut Mesher, e: &Eb, bl: &Blade, near: bool) {
    let th = BLADE_BOARD_M / 2.0;
    let (s, y0, y1, w0, w1) = (bl.s, bl.y0, bl.y1, bl.w0, bl.w1);
    // the board: a field (lit box) or a dark board (neon), its edge frame.
    let board_glow = if bl.neon { 0 } else { bl.glow };
    {
        // (stage S1) a lettered board's faces carry its words (its tubes, on a neon board).
        if let Some(l) = &bl.letters {
            draw_lettered_board(m, e, bl, l);
        } else {
            let sk = if bl.neon { m.at(M_BOARD, 0xffffff, 0) } else { m.col(M_PANEL, bl.field, board_glow) };
            ebox(sk, e, s - th, s + th, y0, y1, w0, w1, F_S0 | F_S1);
        }
        let fr = steel_finish([40, 40, 40]);
        let sk = m.at(fr.0, fr.1, 0);
        ebox(sk, e, s - th - 0.005, s + th + 0.005, y0 - 0.03, y0, w0 - 0.02, w1 + 0.02, F_ALL);
        ebox(sk, e, s - th - 0.005, s + th + 0.005, y1, y1 + 0.03, w0 - 0.02, w1 + 0.02, F_ALL);
        ebox(sk, e, s - th - 0.005, s + th + 0.005, y0, y1, w1, w1 + 0.02, F_OUT | F_S0 | F_S1);
        ebox(sk, e, s - th - 0.005, s + th + 0.005, y0, y1, w0 - 0.02, w0, F_IN | F_S0 | F_S1);
        // the bracket: an arm from the wall over the board and two hangers.
        ebox(sk, e, s - 0.02, s + 0.02, y1 + 0.03, y1 + 0.09, 0.0, w1 + 0.04, F_NO_BACK);
        ebox(sk, e, s - 0.02, s + 0.02, y0 - 0.09, y0 - 0.03, 0.0, w0 + 0.02, F_NO_BACK);
        ebox(sk, e, s - 0.08, s + 0.08, y0 - 0.12, y1 + 0.12, 0.0, 0.012, F_OUT | F_S0 | F_S1 | F_TOP | F_BOT);
    }
    // both faces: shapes laid along the board (x along w, y up).
    let (h, wd) = (y1 - y0, w1 - w0);
    let c_w = (w0 + w1) / 2.0;
    let c_y = (y0 + y1) / 2.0;
    if bl.letters.is_some() {
        // (stage S1) the words, tubes and border are drawn on the faces (draw.wgsl `sl_sign`).
    } else if bl.neon {
        // neon tubes: an outline and a shape or two, on both faces.
        let tubes = bl.tubes;
        let sk = m.col(M_PANEL, tubes, bl.glow);
        for side in [-1.0f64, 1.0] {
            let off = side * (th + 0.025);
            let q = |w: f64, y: f64| e.p(s + off, y, w);
            let (a, b) = (0.07, 0.07);
            let pts = [q(w0 + a, y0 + b), q(w1 - a, y0 + b), q(w1 - a, y1 - b), q(w0 + a, y1 - b)];
            for i in 0..4 {
                cyl(sk, pts[i], pts[(i + 1) % 4], 0.009, 6, false);
            }
            if near {
                // a ring with rays (a sunburst) and, on a tall board, a diamond: emblems, no glyphs.
                let r = (wd * 0.22).min(h * 0.18);
                let cc = (c_w, y1 - b - r * 1.7 - 0.06);
                let n = 12;
                for i in 0..n {
                    let a0 = std::f64::consts::TAU * i as f64 / n as f64;
                    let a1 = std::f64::consts::TAU * (i + 1) as f64 / n as f64;
                    cyl(sk, q(cc.0 + r * a0.cos(), cc.1 + r * a0.sin()), q(cc.0 + r * a1.cos(), cc.1 + r * a1.sin()), 0.008, 6, false);
                    if i % 2 == 0 {
                        cyl(sk, q(cc.0 + r * 1.25 * a0.cos(), cc.1 + r * 1.25 * a0.sin()), q(cc.0 + r * 1.6 * a0.cos(), cc.1 + r * 1.6 * a0.sin()), 0.007, 6, false);
                    }
                }
                let (dy0, dy1) = (y0 + b + 0.08, cc.1 - r * 1.6 - 0.1);
                if dy1 - dy0 > 0.3 {
                    let (my, hr, hw) = ((dy0 + dy1) / 2.0, (dy1 - dy0) / 2.0, (wd / 2.0 - a - 0.06).min((dy1 - dy0) * 0.35));
                    let d = [q(c_w, my - hr), q(c_w + hw, my), q(c_w, my + hr), q(c_w - hw, my)];
                    for i in 0..4 {
                        cyl(sk, d[i], d[(i + 1) % 4], 0.008, 6, false);
                    }
                }
            }
        }
    } else if near {
        for side in [-1.0f64, 1.0] {
            let o = e.p(s + side * th, c_y, c_w);
            let ax = e.dn();
            let ay = [0.0, 1.0, 0.0];
            let n = scale(e.du(), side);
            // a basis whose normal points out of this face.
            let (ax, ay) = if dot3(cross(ax, ay), n) > 0.0 { (ax, ay) } else { (neg(ax), ay) };
            // the design was laid out along the board's height: turn it upright.
            let shapes: Vec<Shape2> = bl.shapes.iter().map(|sh| Shape2 { c: [sh.c[1], sh.c[0]], r: [sh.r[1], sh.r[0]], ..sh.clone() }).collect();
            let shapes: Vec<Shape2> = shapes.into_iter().filter(|sh| sh.c[0].abs() + sh.r[0] < wd / 2.0 + 0.02 && sh.c[1].abs() + sh.r[1] < h / 2.0 + 0.02).collect();
            draw_shapes(m, o, ax, ay, &shapes, 0.0, 0.004, bl.glow, false);
        }
    }
}

fn dot3(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn draw_shutter(m: &mut Mesher, e: &Eb, sh: &Shutter, near: bool) {
    let fin = finish_for(sh.colour, M_GALV);
    let paint = tint(fin, sh.colour);
    let sk = m.at(fin, paint, 0);
    let (s0, s1, w0) = (sh.s0, sh.s1, sh.w0);
    // the hood (coil box) over the head.
    ebox(sk, e, s0 - 0.07, s1 + 0.07, sh.head, sh.head + 0.32, w0, w0 + 0.28, F_NO_BACK);
    if near {
        // the guides at both sides.
        for (a, b) in [(s0 - 0.07, s0), (s1, s1 + 0.07)] {
            ebox(sk, e, a, b, sh.floor, sh.head, w0, w0 + 0.09, F_NO_BACK);
        }
    }
    if sh.down {
        let (cw0, cw1) = (w0 + 0.03, w0 + 0.05);
        ebox(sk, e, s0, s1, sh.floor + 0.01, sh.head, cw0, cw1, F_OUT | F_IN | F_BOT);
        if near {
            // the curtain's slats: a lip every slat pitch, and the bottom bar.
            let dark = m.at(fin, tint(fin, shade(sh.colour, 0.72)), 0);
            let mut y = sh.floor + 0.09;
            while y < sh.head - 0.02 {
                ebox(dark, e, s0, s1, y - 0.012, y, cw1, cw1 + 0.008, F_OUT | F_TOP | F_BOT);
                y += 0.085;
            }
            ebox(dark, e, s0, s1, sh.floor + 0.01, sh.floor + 0.07, cw1, cw1 + 0.025, F_OUT | F_TOP | F_S0 | F_S1);
        }
    }
}

fn draw_grille(m: &mut Mesher, e: &Eb, g: &Grille) {
    let f = steel_finish(g.colour);
    let sk = m.at(f.0, f.1, 0);
    let (w0, w1) = (g.w0, g.w0 + 0.026);
    // the frame of flat bar, two rails and the pickets.
    ebox(sk, e, g.s0, g.s1, g.y0, g.y0 + 0.03, w0, w1, F_NO_BACK);
    ebox(sk, e, g.s0, g.s1, g.y1 - 0.03, g.y1, w0, w1, F_NO_BACK);
    ebox(sk, e, g.s0, g.s0 + 0.03, g.y0 + 0.03, g.y1 - 0.03, w0, w1, F_ALONG_Y);
    ebox(sk, e, g.s1 - 0.03, g.s1, g.y0 + 0.03, g.y1 - 0.03, w0, w1, F_ALONG_Y);
    for t in [0.33, 0.67] {
        let y = g.y0 + (g.y1 - g.y0) * t;
        ebox(sk, e, g.s0 + 0.03, g.s1 - 0.03, y - 0.012, y + 0.012, w0 + 0.004, w1 - 0.012, F_ALONG_S);
    }
    let n = (((g.s1 - g.s0) - 0.06) / 0.115).round().max(2.0) as usize;
    for i in 1..n {
        let s = g.s0 + 0.03 + (g.s1 - g.s0 - 0.06) * i as f64 / n as f64;
        ebox(sk, e, s - 0.008, s + 0.008, g.y0 + 0.03, g.y1 - 0.03, w1 - 0.02, w1 - 0.004, F_ALONG_Y);
    }
}

fn draw_boarding(m: &mut Mesher, e: &Eb, bd: &Boarding, near: bool) {
    // plywood sheets (4 x 8 ft) with gaps, a sheet a roll of tone.
    let mut s = bd.s0;
    let mut k = 0u32;
    while s < bd.s1 - 0.05 {
        let b = (s + 1.219).min(bd.s1);
        let tone = shade(PLYWOOD, 0.88 + 0.14 * ((k * 2654435761) % 1000) as f64 / 1000.0);
        let sk = m.col(M_PANEL, tone, 0);
        let gap = if near { 0.006 } else { 0.0 };
        ebox(sk, e, s + gap, b - gap, bd.y0, bd.y1, bd.w0, bd.w0 + 0.019, F_NO_BACK);
        s = b;
        k += 1;
    }
}

fn draw_downspout(m: &mut Mesher, e: &Eb, ds: &Downspout, mat: &str, paint: u32, near: bool) {
    let sk = m.at(mat, paint, 0);
    let (a, b) = (ds.s - DS_W_M / 2.0, ds.s + DS_W_M / 2.0);
    let (w0, w1) = (ds.w0, ds.w0 + DS_D_M);
    for (y0, y1) in &ds.runs {
        ebox(sk, e, a, b, *y0, *y1, w0, w1, F_ALONG_Y | F_BOT);
    }
    if near {
        // the straps fixing it to the wall.
        for y in &ds.straps {
            ebox(sk, e, a - 0.012, b + 0.012, *y, y + 0.03, 0.0, w1 + 0.006, F_NO_BACK);
        }
    }
    if let Some(h) = ds.head {
        // the conductor head: a box narrowing to the pipe.
        let prof = vec![[0.0, h + 0.3], [w0 + 0.19, h + 0.3], [w0 + 0.17, h + 0.08], [w1, h], [w0, h], [0.0, h + 0.05]];
        sweep(sk, e, &prof, ds.s - 0.15, ds.s + 0.15, true);
    }
}

fn draw_meters(m: &mut Mesher, e: &Eb, mt: &Meters, drawn: &DrawnMaterials) {
    let (fin, paint) = paint_of(mt.paint, drawn);
    let fin = fin.as_str();
    let w0 = mt.w0;
    let mut s = mt.s;
    let y_m = mt.floor + 1.25;
    // electric meters: sockets in a row with their glass, the conduit up the wall.
    for _ in 0..mt.electric {
        let sk = m.at(fin, paint, 0);
        ebox(sk, e, s, s + 0.26, y_m, y_m + 0.36, w0, w0 + 0.12, F_NO_BACK);
        let glass = m.col(M_PANEL, [196, 200, 198], 0);
        cyl(glass, e.p(s + 0.13, y_m + 0.2, w0 + 0.12), e.p(s + 0.13, y_m + 0.2, w0 + 0.19), 0.075, 10, true);
        s += 0.3;
    }
    {
        let cg = steel_finish(GALV);
        let sk = m.at(cg.0, cg.1, 0);
        let cs = mt.s + 0.13;
        cyl(sk, e.p(cs, y_m + 0.36, w0 + 0.05), e.p(cs, mt.conduit_top, w0 + 0.05), 0.024, 8, false);
        ebox(sk, e, cs - 0.05, cs + 0.05, mt.conduit_top - 0.04, mt.conduit_top + 0.06, w0, w0 + 0.09, F_NO_BACK);
        // the service conduit down into the sidewalk.
        cyl(sk, e.p(mt.s + 0.13 + 0.3 * (mt.electric as f64 - 1.0), mt.floor - 0.05, w0 + 0.045), e.p(mt.s + 0.13 + 0.3 * (mt.electric as f64 - 1.0), y_m, w0 + 0.045), 0.03, 8, false);
    }
    if mt.gas {
        // a gas meter set: riser from the ground, the meter, the regulator, into the wall.
        let gs = s + 0.15;
        let sk = m.at(fin, paint, 0);
        let gy = mt.floor + 0.3;
        ebox(sk, e, gs, gs + 0.28, gy, gy + 0.3, w0 + 0.05, w0 + 0.27, F_NO_BACK);
        let pg = steel_finish([96, 100, 100]);
        let pk = m.at(pg.0, pg.1, 0);
        cyl(pk, e.p(gs + 0.04, mt.floor - 0.05, w0 + 0.16), e.p(gs + 0.04, gy, w0 + 0.16), 0.022, 8, false);
        cyl(pk, e.p(gs + 0.24, gy + 0.3, w0 + 0.16), e.p(gs + 0.24, gy + 0.55, w0 + 0.16), 0.02, 8, false);
        cyl(pk, e.p(gs + 0.24, gy + 0.55, w0 + 0.16), e.p(gs + 0.24, gy + 0.55, w0), 0.02, 8, false);
        cyl(pk, e.p(gs + 0.14, gy + 0.36, w0 + 0.16), e.p(gs + 0.14, gy + 0.36, w0 + 0.26), 0.05, 8, true);
    }
    if let Some((fs, c)) = mt.fdc {
        // the fire department connection: a body and two capped inlets.
        let sk = m.col(M_PANEL, c, 0);
        let y = mt.floor + 0.8;
        cyl(sk, e.p(fs - 0.2, y, 0.08), e.p(fs + 0.2, y, 0.08), 0.06, 10, true);
        for ds in [-0.13, 0.13] {
            cyl(sk, e.p(fs + ds, y, 0.08), e.p(fs + ds, y - 0.02, 0.24), 0.045, 10, true);
        }
        cyl(sk, e.p(fs, y, 0.0), e.p(fs, y, 0.08), 0.05, 10, false);
        let plate = m.col(M_PANEL, shade(c, 0.8), 0);
        ebox(plate, e, fs - 0.26, fs + 0.26, y - 0.14, y + 0.14, 0.0, 0.012, F_NO_BACK);
    }
}

fn draw_ac(m: &mut Mesher, e: &Eb, ac: &WindowAc) {
    let sk = m.col(M_PANEL, AC_BEIGE, 0);
    ebox(sk, e, ac.s0, ac.s1, ac.sill, ac.sill + 0.38, -0.12, 0.4, F_NO_BACK);
    let g = m.col(M_PANEL, shade(AC_BEIGE, 0.55), 0);
    let mut y = ac.sill + 0.06;
    while y < ac.sill + 0.33 {
        ebox(g, e, ac.s0 + 0.04, ac.s1 - 0.04, y, y + 0.012, 0.4, 0.41, F_OUT | F_TOP | F_BOT);
        y += 0.04;
    }
    // the support bracket under it.
    let br = steel_finish([60, 60, 60]);
    let sk = m.at(br.0, br.1, 0);
    for s in [ac.s0 + 0.06, ac.s1 - 0.06] {
        bar(sk, e.p(s, ac.sill - 0.35, 0.0), e.p(s, ac.sill - 0.01, 0.36), 0.01, 0.015, e.du(), false);
    }
}

fn draw_dish(m: &mut Mesher, d: &Dish, near: bool) {
    let sk = m.col(M_PANEL, DISH_WHITE, 0);
    let n = d.look;
    let depth = d.r * 0.28;
    // a shallow cone (the reflector) and its back.
    let mut x = cross(n, [0.0, 1.0, 0.0]);
    if len(x) < 1e-6 {
        x = [1.0, 0.0, 0.0];
    }
    let x = norm(x);
    let y = cross(x, n);
    let segs = if near { 14 } else { 8 };
    let rim: Vec<V3> = (0..segs)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / segs as f64;
            add(d.at, add(scale(x, d.r * a.cos()), scale(y, d.r * 0.92 * a.sin())))
        })
        .collect();
    let apex = sub(d.at, scale(n, depth));
    for i in 0..segs {
        let j = (i + 1) % segs;
        let fnorm = norm(cross(sub(rim[j], apex), sub(rim[i], apex)));
        let base = sk.positions.len() as u32;
        for (p, nn) in [(apex, fnorm), (rim[i], fnorm), (rim[j], fnorm)] {
            sk.positions.push(p);
            sk.normals.push([nn[0] as f32, nn[1] as f32, nn[2] as f32]);
        }
        sk.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    let grey = steel_finish([110, 112, 112]);
    let gk = m.at(grey.0, grey.1, 0);
    if near {
        // the feed arm and the LNB at the focus.
        let focus = add(d.at, scale(n, d.r * 0.85));
        let lower = add(d.at, scale(y, -d.r * 0.85));
        rod(gk, lower, focus, 0.012);
        ebox_world(gk, focus, 0.04);
    }
    // the mount: a mast to the roof deck, or an arm to the wall.
    match d.mount {
        None => {
            let foot = [d.at[0], d.at[1] - 0.75, d.at[2]];
            cyl(gk, foot, apex, 0.025, 6, false);
            if near {
                // the non-penetrating base frame.
                bar(gk, add(foot, [-0.4, 0.0, 0.0]), add(foot, [0.4, 0.0, 0.0]), 0.03, 0.02, [0.0, 1.0, 0.0], true);
                bar(gk, add(foot, [0.0, 0.0, -0.4]), add(foot, [0.0, 0.0, 0.4]), 0.03, 0.02, [0.0, 1.0, 0.0], true);
            }
        }
        Some(wall) => {
            // an arm from a wall plate to the reflector's back.
            rod(gk, wall, apex, 0.02);
            if near {
                let up = [wall[0], wall[1] + 0.12, wall[2]];
                rod(gk, up, apex, 0.012);
            }
        }
    }
}

/// A small cube (world-aligned) at `c`.
fn ebox_world(sk: &mut Sink, c: V3, h: f64) {
    let e = Eb { o: [c[0], c[2]], u: [1.0, 0.0], n: [0.0, 1.0] };
    ebox(sk, &e, -h, h, c[1] - h, c[1] + h, -h, h, F_ALL);
}

fn draw_scaffold(m: &mut Mesher, e: &Eb, sc: &Scaffold, near: bool) {
    let pipe = steel_finish([150, 152, 150]);
    let plank = steel_finish([150, 120, 80]);
    let (wa, wb) = (sc.w0, sc.w0 + 1.05);
    let bays = ((sc.s1 - sc.s0) / 2.1).ceil().max(1.0) as usize;
    let lifts = ((sc.top - sc.floor) / 2.0).floor().max(1.0) as usize;
    let sk = m.at(pipe.0, pipe.1, 0);
    for i in 0..=bays {
        let s = sc.s0 + (sc.s1 - sc.s0) * i as f64 / bays as f64;
        for w in [wa, wb] {
            cyl(sk, e.p(s, sc.floor, w), e.p(s, sc.top, w), 0.024, if near { 8 } else { 4 }, false);
        }
    }
    for j in 1..=lifts {
        let y = sc.floor + 2.0 * j as f64;
        for w in [wa, wb] {
            cyl(sk, e.p(sc.s0, y, w), e.p(sc.s1, y, w), 0.024, if near { 8 } else { 4 }, false);
        }
        if near {
            // the guard rail on the outside and the toe board.
            cyl(sk, e.p(sc.s0, y + 1.0, wb), e.p(sc.s1, y + 1.0, wb), 0.02, 6, false);
            for i in 0..bays {
                let s = sc.s0 + (sc.s1 - sc.s0) * i as f64 / bays as f64;
                let s2 = sc.s0 + (sc.s1 - sc.s0) * (i + 1) as f64 / bays as f64;
                cyl(sk, e.p(s, y, wb), e.p(s2, y + 2.0, wb), 0.018, 6, false);
            }
        }
    }
    let pk = m.at(plank.0, plank.1, 0);
    for j in 1..=lifts {
        let y = sc.floor + 2.0 * j as f64;
        ebox(pk, e, sc.s0, sc.s1, y + 0.025, y + 0.065, wa + 0.03, wb - 0.03, F_ALL);
    }
}

fn draw_roof(m: &mut Mesher, r: &Roof, near: bool) {
    match r {
        Roof::Chimney { c, u, half, y0, y1, pots } => {
            let e = Eb { o: *c, u: *u, n: [-u[1], u[0]] };
            let brick = m.at(M_BRICK, 0xf2ece6, 0);
            ebox(brick, &e, -half[0], half[0], *y0, *y1, -half[1], half[1], F_ALL & !F_BOT);
            let stone = m.at(M_STONE, 0xd8d4cc, 0);
            ebox(stone, &e, -half[0] - 0.05, half[0] + 0.05, *y1, y1 + 0.08, -half[1] - 0.05, half[1] + 0.05, F_ALL);
            if near {
                let clay = m.at(M_CLAY, 0xe0d0c8, 0);
                for k in 0..*pots {
                    let t = if *pots == 1 { 0.0 } else { -half[0] * 0.5 + half[0] * k as f64 / (*pots - 1).max(1) as f64 };
                    let p = e.p(t, y1 + 0.08, 0.0);
                    cyl(clay, p, [p[0], p[1] + 0.38, p[2]], 0.1, 8, true);
                }
            }
        }
        Roof::Vent { c, r, y0, y1, cap, dark } => {
            if !near {
                return;
            }
            let f = if *dark { steel_finish([46, 46, 46]) } else { steel_finish(GALV) };
            let sk = m.at(f.0, f.1, 0);
            cyl(sk, [c[0], *y0, c[1]], [c[0], *y1, c[1]], *r, 8, true);
            if *cap {
                cyl(sk, [c[0], *y1, c[1]], [c[0], y1 + 0.08, c[1]], r * 2.2, 8, true);
            }
        }
        Roof::Aerial { c, y0, h, yaw, elements } => {
            if !near {
                return;
            }
            let f = steel_finish([120, 122, 122]);
            let sk = m.at(f.0, f.1, 0);
            let top = y0 + h;
            cyl(sk, [c[0], *y0, c[1]], [c[0], top, c[1]], 0.02, 6, false);
            let (dx, dz) = (yaw.cos(), yaw.sin());
            let boom = 0.75;
            let a = [c[0] - dx * boom, top - 0.1, c[1] - dz * boom];
            let b = [c[0] + dx * boom, top - 0.1, c[1] + dz * boom];
            rod(sk, a, b, 0.01);
            for k in 0..*elements {
                let t = -boom + 2.0 * boom * k as f64 / (*elements - 1).max(1) as f64;
                let half = 0.55 - 0.3 * (k as f64 / *elements as f64);
                let p = [c[0] + dx * t, top - 0.1, c[1] + dz * t];
                rod(sk, [p[0] + dz * half, p[1], p[2] - dx * half], [p[0] - dz * half, p[1], p[2] + dx * half], 0.006);
            }
        }
    }
}

/// The street faces the planner dresses and what already stands out of each
/// (the plan's own parts, from any edge, in that face's basis), with the
/// most a piece may stand out of it: (edge index, length, reach, obstacles).
/// For the tests and tools; the planner works from the same.
pub fn face_obstacles(planned: &Planned) -> Vec<(usize, f64, f64, Vec<Bx>)> {
    faces(planned, limits()).into_iter().map(|f| (f.edge.index, f.edge.length, f.reach, f.obstacles)).collect()
}

/// Plan and draw a building's clutter for one rung (`pipeline::build_rung_with`).
pub fn add_to_rung(planned: &Planned, rung: u32, level: u8, drawn: &DrawnMaterials, out: &mut RungMesh) -> ClutterPlan {
    let cp = plan(planned, level);
    emit(&cp, rung, drawn, out);
    cp
}
