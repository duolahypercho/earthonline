//! The flora pass around the Golden Gate (`--no-flora` restores the third
//! grass pass's output): the coastal scrub by species, each with its own
//! growth form, leaf and colour, placed by the ground's shape.
//!
//! Source of truth: the Golden Gate field guide (species table, seasons,
//! placement), itself citing the NPS Golden Gate "Scrublands" page and the
//! Calscape (CNPS) species pages. Numbers marked REF are typical botanical
//! values the guide did not check against its source; UNVERIFIED are photo
//! readings no source confirms; "estimate" values are the guide's estimates
//! from the photographs. Measured values say so ([P:spencer] is the Battery
//! Spencer photograph, 13 April 2018).

use crate::vegetation::{field, hash, smoothstep, warped, Site, MEADOW_COYOTE, MEADOW_LUPINE, MEADOW_SAGEBRUSH};

/// The flora pass's switch names (the roll-up first); eo-app accepts them,
/// [`Flora::from_args`] reads them.
pub const FLORA_SWITCHES: &[&str] = &["--no-flora", "--no-scrub-species", "--no-grass-ramp", "--no-bluff-scrub", "--no-chert-ground", "--no-serpentinite"];

/// The flora pass's features, each with its `--no-` switch ([`FLORA_SWITCHES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flora {
    /// `--no-scrub-species`: the scrub as its species (coyote brush, California
    /// sagebrush, poison oak, sticky monkeyflower, coffeeberry, toyon, bush
    /// lupine), each its own form, leaf sprigs and colour, in the gullies, lee
    /// sides and lower slopes; grassland keeps the ridges and convex shoulders.
    pub species: bool,
    /// `--no-grass-ramp`: the annual grassland follows the guide's month table
    /// ([`grass_cure`]: green in winter and spring, the first annuals yellowing
    /// in April, straw by June, bleached grey-tan in September and October) and
    /// carries seed stalks from March ([`MEADOW_SEED_STALKS`]).
    pub ramp: bool,
    /// `--no-bluff-scrub`: the San Francisco serpentine bluffs carry low coast bluff
    /// scrub (coast buckwheat, seaside daisy, Douglas iris, dudleya) among red fescue
    /// instead of the headlands' meadow, the coyote brush sparse.
    pub bluff: bool,
    /// `--no-chert-ground`: the Marin ground's bare soil is red chert soil and rubble at
    /// the guide's measured colours ([`CHERT_TOP_SRGB`], [`CHERT_FACE_SRGB`]), steep
    /// Marin faces carry ribbon-chert outcrops (folded thin beds), and the bare rule
    /// shrinks to the knoll tops, scars and faces (the overlook's shoulders are scrub).
    pub chert: bool,
    /// `--no-serpentinite`: the Presidio's serpentinite: the bluffs' rock as blue-green,
    /// scaly, polished knobs 0.5-3 m standing out of the sheared matrix (not 4.5-9.5 m
    /// pillows), and rounded dark serpentinite and greenstone boulders 0.5-3 m (a few
    /// to 5 m) in the surf and on the sand at Baker and Marshall's, as the S1 scans.
    pub serpentinite: bool,
}

impl Flora {
    pub const ALL: Flora = Flora { species: true, ramp: true, bluff: true, chert: true, serpentinite: true };
    pub const NONE: Flora = Flora { species: false, ramp: false, bluff: false, chert: false, serpentinite: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-flora") {
            return Self::NONE;
        }
        Flora { species: !has("--no-scrub-species"), ramp: !has("--no-grass-ramp"), bluff: !has("--no-bluff-scrub"), chert: !has("--no-chert-ground"), serpentinite: !has("--no-serpentinite") }
    }
}

/// The flora pass's switches of this process (read once from the command
/// line). It acts inside the third grass pass's scrub mounds: without them
/// (`--no-scrub-mounds`, `--no-grass3`) the species have nothing to replace.
pub fn flora() -> Flora {
    // the crate's unit tests read the pass explicitly (`Extra::fl`, `Flora::from_args`):
    // the test runner's arguments must not switch it on under the round's own tests.
    if cfg!(test) {
        return Flora::NONE;
    }
    static S: std::sync::OnceLock<Flora> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        let f = Flora::from_args(&std::env::args().collect::<Vec<_>>());
        let g3 = crate::vegetation::grass3();
        // the species replace the third pass's mounds; the grassland's ramp and the bluff
        // scrub act in the second pass's meadow (the third pass keeps it when on).
        Flora { species: f.species && g3.scrub, ramp: f.ramp && g3 != crate::vegetation::Grass3::NONE, bluff: f.bluff && g3 != crate::vegetation::Grass3::NONE, chert: f.chert && g3.bare, serpentinite: f.serpentinite && g3 != crate::vegetation::Grass3::NONE }
    })
}

// ------------------------------------------------------------------ species

/// Poison oak, *Toxicodendron diversilobum* (meadow model index; after the third pass's types).
pub const MEADOW_POISON_OAK: u8 = 10;
/// Sticky monkeyflower, *Diplacus aurantiacus*.
pub const MEADOW_MONKEYFLOWER: u8 = 11;
/// California coffeeberry, *Frangula californica*.
pub const MEADOW_COFFEEBERRY: u8 = 12;
/// Toyon, *Heteromeles arbutifolia*.
pub const MEADOW_TOYON: u8 = 13;

/// Whether a meadow type is a scrub mound (a shrub, drawn with leaf sprigs under the flora pass).
pub fn is_shrub(kind: u8) -> bool {
    (MEADOW_COYOTE..=MEADOW_LUPINE).contains(&kind) || (MEADOW_POISON_OAK..=MEADOW_TOYON).contains(&kind)
}

/// The leaf sprig a species' cards carry: a cell of the vegetation atlas
/// (`ez-vegetation-atlas-v1`, drawn grey and coloured per species).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sprig {
    /// Round-ovate small leaves on a twig (coyote brush, lupine).
    Round,
    /// Fine needle-like leaves (the sagebrush's threadlike leaves).
    Fine,
    /// Lobed leaves (poison oak's lobed leaflets).
    Lobed,
    /// Oval leaflets along a stem (coffeeberry, toyon; narrowed: monkeyflower).
    Oval,
}

/// A species' growth form at model scale 1 (game metres; the planting scales it).
#[derive(Clone, Copy, Debug)]
pub struct Form {
    /// Height and radius of the shrub's model.
    pub h: f32,
    pub r: f32,
    /// Leaf masses (displaced spheres) in the near level.
    pub clumps: u32,
    /// Crown shape: the clumps' vertical stretch (under 1 a flat mound, over 1 upright).
    pub upright: f32,
    pub sprig: Sprig,
    /// A sprig card's side (m) in the near level, and its width over its height.
    pub card: f32,
    pub aspect: f32,
    /// Sprig cards per level (near, mid, far).
    pub cards: [u32; 3],
    /// How far the sprigs stand out of the clumps (1: at the surface).
    pub reach: f32,
}

/// A species' sunlit near-field foliage in April (sRGB) and the planted
/// height range at model scale (instance scale = height / `Form::h`).
#[derive(Clone, Copy, Debug)]
pub struct Species {
    pub kind: u8,
    pub name: &'static str,
    pub leaf_srgb: [u8; 3],
    pub form: Form,
    /// The planted height range (game m).
    pub height: (f32, f32),
}

/// The seven scrub species (field guide section 2).
pub const SPECIES: [Species; 7] = [
    // Coyote brush: evergreen, 0.6-3 m [S20], prostrate on windy bluffs (REF); leaves
    // 1-4 cm obovate, toothed, glossy (REF). Colour: the measured near-field green
    // (90,103,43) [P:spencer] is the rendered target: the albedo here is tuned so the
    // headland's sunlit near field renders it. 2 m: a dense twiggy mound of small bright leaves.
    Species { kind: MEADOW_COYOTE, name: "coyote brush", leaf_srgb: [104, 118, 44], form: Form { h: 1.0, r: 0.8, clumps: 12, upright: 0.85, sprig: Sprig::Round, card: 0.24, aspect: 1.0, cards: [620, 220, 70], reach: 1.0 }, height: (0.55, 1.9) },
    // California sagebrush: summer-deciduous, 0.3-2.4 m [S21]; threadlike silvery leaves
    // (1-4 cm REF). Silver grey-green (120-160, 135-165, 110-140) (estimate), its low
    // end as albedo: the range is the photographs' sunlit pixels, and at its middle
    // the sage drew as white blossom beside the coyote brush.
    Species { kind: MEADOW_SAGEBRUSH, name: "California sagebrush", leaf_srgb: [118, 128, 104], form: Form { h: 0.9, r: 0.7, clumps: 9, upright: 0.9, sprig: Sprig::Fine, card: 0.3, aspect: 0.9, cards: [520, 180, 60], reach: 1.0 }, height: (0.45, 1.4) },
    // Yellow bush lupine: evergreen, 1.2-2.1 m [S22]; palmate grey-green silky leaves,
    // (100-140, 120-150, 90-120) (estimate).
    Species { kind: MEADOW_LUPINE, name: "bush lupine", leaf_srgb: [118, 135, 105], form: Form { h: 1.1, r: 0.8, clumps: 10, upright: 0.9, sprig: Sprig::Round, card: 0.3, aspect: 1.0, cards: [520, 180, 60], reach: 1.05 }, height: (0.8, 1.7) },
    // Poison oak: winter-deciduous shrub or vine, 0.6-4 m [S27]; three lobed glossy
    // leaflets 3-10 cm (REF); fresh green (80-120, 110-140, 40-70) (estimate), low and
    // sprawling in the scrub.
    Species { kind: MEADOW_POISON_OAK, name: "poison oak", leaf_srgb: [100, 125, 55], form: Form { h: 0.9, r: 0.95, clumps: 10, upright: 0.7, sprig: Sprig::Lobed, card: 0.38, aspect: 1.0, cards: [460, 160, 55], reach: 1.1 }, height: (0.5, 1.4) },
    // Sticky monkeyflower: subshrub about 1.2-1.5 m [S24]; narrow dark glossy leaves
    // 3-7 cm (REF), (50-80, 70-100, 30-50) (estimate); upright leafy stems.
    Species { kind: MEADOW_MONKEYFLOWER, name: "sticky monkeyflower", leaf_srgb: [65, 85, 40], form: Form { h: 1.0, r: 0.5, clumps: 7, upright: 1.3, sprig: Sprig::Oval, card: 0.42, aspect: 0.55, cards: [380, 130, 45], reach: 1.1 }, height: (0.6, 1.3) },
    // Coffeeberry: 1.8-4.5 m [S26]; oval dark leaves with a red tint, 3-8 cm (REF),
    // (40-70, 60-85, 30-50) (estimate); darker, taller mounds than coyote brush.
    Species { kind: MEADOW_COFFEEBERRY, name: "coffeeberry", leaf_srgb: [56, 72, 40], form: Form { h: 1.8, r: 1.1, clumps: 14, upright: 0.95, sprig: Sprig::Oval, card: 0.55, aspect: 1.0, cards: [560, 190, 65], reach: 1.0 }, height: (1.3, 2.6) },
    // Toyon: 2-6 m (REF); toothed oblong leaves 5-10 cm (REF), dark green (estimate).
    // Wind keeps it low on the headland: a rounded shrub over the scrub (INFERRED).
    Species { kind: MEADOW_TOYON, name: "toyon", leaf_srgb: [50, 70, 40], form: Form { h: 2.4, r: 1.2, clumps: 14, upright: 1.1, sprig: Sprig::Oval, card: 0.75, aspect: 1.0, cards: [520, 180, 65], reach: 1.0 }, height: (1.8, 3.0) },
];

/// A scrub species by its meadow type.
pub fn species(kind: u8) -> Option<&'static Species> {
    SPECIES.iter().find(|s| s.kind == kind)
}

/// The leaf colour as linear albedo.
pub fn leaf_linear(kind: u8) -> [f32; 3] {
    let c = species(kind).map(|s| s.leaf_srgb).unwrap_or([90, 103, 43]);
    c.map(|v| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    })
}

/// Poison oak's leaves by the day of the year (a factor on its April colour):
/// red to orange from late summer, (150-190, 50-90, 30-50) in Sep-Oct (estimate,
/// [S27] "vivid"); leafless grey twigs in Dec-Jan (guide's season table).
pub fn poison_oak_season(day_of_year: f64) -> [f32; 3] {
    let d = day_of_year.rem_euclid(365.0);
    // turning from mid-August to the end of September, leaves falling in December.
    let red = smoothstep(225.0, 270.0, d) * (1.0 - smoothstep(320.0, 345.0, d));
    let bare = smoothstep(330.0, 350.0, d).max(1.0 - smoothstep(20.0, 45.0, d));
    // (170,70,40) over (100,125,55), linear: (3.1, 0.30, 0.56).
    let r = [1.0 + 2.1 * red as f32, 1.0 - 0.7 * red as f32, 1.0 - 0.44 * red as f32];
    // the grey of bare twigs over the green, linear (about 0.55 grey-brown, INFERRED).
    let g = [1.25f32, 0.62, 1.4];
    [0, 1, 2].map(|i| r[i] + (g[i] - r[i]) * bare as f32)
}

// ------------------------------------------------------------------ the annual grassland

/// Seed stalks of the annual grasses (wild oats, ripgut brome): tall culms with
/// nodding panicles standing over the meadow, no mat (meadow model index).
pub const MEADOW_SEED_STALKS: u8 = 14;
/// Coast buckwheat, *Eriogonum latifolium*: woolly pale cushions about 0.6 m [S30].
pub const MEADOW_BUCKWHEAT: u8 = 15;
/// The bluff's low herbs: seaside daisy mats, Douglas iris fans and live-forever
/// (dudleya) rosettes [S6], over thin turf.
pub const MEADOW_BLUFF_HERBS: u8 = 16;

/// How cured the annual grassland is on a day of the year (0 green .. 1
/// cured), the field guide's month table (section 3) read at mid-month:
/// Dec-Mar green (the first flush in November gold with a green cast);
/// April green with seed heads up and the first annuals yellowing on thin
/// south-facing soil (estimate); May curing on south faces and thin soils
/// (estimate); June most cured, patchy; July-August uniform gold-tan;
/// September-October bleached, grey-tan. Mid-month values INFERRED from the
/// table's words.
pub fn grass_cure(day_of_year: f64) -> f64 {
    const T: [(f64, f64); 13] = [(15.0, 0.03), (46.0, 0.0), (74.0, 0.0), (105.0, 0.12), (135.0, 0.42), (166.0, 0.72), (196.0, 0.9), (227.0, 0.96), (258.0, 1.0), (288.0, 1.0), (319.0, 0.55), (349.0, 0.1), (380.0, 0.03)];
    let d = day_of_year.rem_euclid(365.0);
    let d = if d < T[0].0 { d + 365.0 } else { d };
    for w in T.windows(2) {
        if d >= w[0].0 && d <= w[1].0 {
            let t = (d - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + (w[1].1 - w[0].1) * t;
        }
    }
    T[0].1
}

/// How bleached the cured grass is (0 .. 1): September and October's grey-tan,
/// flattened (guide section 3), before the November flush.
pub fn grass_bleach(day_of_year: f64) -> f64 {
    let d = day_of_year.rem_euclid(365.0);
    smoothstep(235.0, 275.0, d) * (1.0 - smoothstep(305.0, 330.0, d))
}

/// A cured patch's colour bleached toward grey-tan: the tint (a factor on the
/// type's green-state colour `live`) with its colour desaturated by `bleach`.
pub fn bleached_tint(tint: [f32; 3], live: [f32; 3], bleach: f64) -> [f32; 3] {
    let c = [0, 1, 2].map(|i| tint[i] * live[i]);
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let grey = [y * 1.04, y, y * 0.9];
    let b = (0.4 * bleach) as f32;
    [0, 1, 2].map(|i| ((c[i] + (grey[i] - c[i]) * b) / live[i].max(1e-4)).min(3.9))
}

/// The share of grassland cells carrying seed stalks on a day: heading from
/// March, standing through the summer (dead stalks), flattened by the
/// November rains (guide: Mar "grass heading", Apr "seed heads up", Oct
/// "flattened"; INFERRED share).
pub fn seed_stalks(day_of_year: f64) -> f64 {
    let d = day_of_year.rem_euclid(365.0);
    0.22 * smoothstep(60.0, 95.0, d) * (1.0 - smoothstep(280.0, 310.0, d))
}

/// The serpentine bluff's patch at a cell (`Flora::bluff`): red fescue, coast
/// buckwheat cushions, the low herbs, and thin sward, each in its own patches
/// (field guide 1.4, coast bluff scrub; shares INFERRED; sparse cover, 20-50 %
/// over rock on the faces, estimate).
pub fn bluff_kind(x: f64, z: f64, ix: i64, iz: i64) -> u8 {
    let w = [
        (crate::vegetation::MEADOW_FESCUE, 1.0),
        (MEADOW_BUCKWHEAT, 0.7 * smoothstep(0.35, 0.7, warped(x, z, 7.0, 141))),
        (MEADOW_BLUFF_HERBS, 0.8 * smoothstep(0.35, 0.7, warped(x, z, 6.0, 142))),
        (crate::vegetation::MEADOW_SWARD, 0.35),
    ];
    let sum: f64 = w.iter().map(|v| v.1).sum();
    let mut r = hash(ix, iz, 143) * sum;
    for (k, v) in w {
        if r < v {
            return k;
        }
        r -= v;
    }
    crate::vegetation::MEADOW_FESCUE
}

// ------------------------------------------------------------------ red chert ground

/// Weathered chert soil, the sunlit flat top of the Battery Spencer knoll:
/// median (190,129,75), p10 (148,118,57), p90 (204,138,85) [P:spencer] (sRGB;
/// the rendered target: trampled thin soil and rubble of red radiolarian chert
/// and its shale partings, iron oxide; field guide 4.1, inference UNVERIFIED).
pub const CHERT_TOP_SRGB: [u8; 3] = [190, 129, 75];
/// The knoll's steep cut face: (154,120,74) [P:spencer].
pub const CHERT_FACE_SRGB: [u8; 3] = [154, 120, 74];
/// Fresh red chert in an outcrop: (140-175, 70-95, 55-75) (estimate), its low
/// end taken toward the neutral as the hill rock's chert (hill_rock.rs
/// `CHERT_SRGB`: the full chroma drew salmon under the frame's light).
pub const CHERT_RIBBON_SRGB: [u8; 3] = [128, 80, 64];
/// Under this height (game m) the Marin ground is the waterline cliffs' basalt
/// and greenstone, not chert (the sequence's base, guide 4.1; cliffs 10-40 m
/// high, game heights 1.25 x real).
pub const CHERT_MIN_Y: f64 = 45.0;
/// The share of a cell the overlook's disturbed shoulders carry in scrub and
/// forbs (`Flora::chert`): the ground the old bare rule stripped but the
/// shrunk one keeps (the Battery Spencer foreground: "dense mid-green leafy
/// herbs and shrubs", guide section 0; share INFERRED).
pub const SHOULDER_SCRUB: f64 = 0.65;
/// The shoulder's shrubs' height factor: wind and salt keep the growth low on
/// the headland's exposed shoulders (guide 1.1), coyote brush prostrate at
/// 0.3-0.6 m on windy bluffs (REF); the photograph's foreground band stands
/// below the eye (INFERRED: 0.4 of the species' height).
pub const SHOULDER_LOW: f32 = 0.4;
/// The tip word that asks the props shader for ribbon-chert bedding on a rock
/// block (props_draw.wgsl; the rocks carry no grass tip otherwise).
pub const TIP_RIBBON: u8 = 254;
/// A Marin face this steep or more carries ribbon-chert outcrops (degrees;
/// the guide's chert ribs and fins along crests and cuts, estimate).
pub const RIBBON_SLOPE_DEG: f64 = 38.0;

// ------------------------------------------------------------------ serpentinite

/// Fresh serpentinite, blue-green to grey-green, (90-130, 110-140, 100-130)
/// (estimate; [S47] "blue-green serpentinite rock"): toward its low end, the
/// bluff's knobs (its middle drew pale cyan under the sunset).
pub const SERPENTINITE_SRGB: [u8; 3] = [92, 112, 102];
/// Extra knobs round each bluff outcrop cell (the sheared matrix's hard blocks).
pub const SERP_KNOBS: u32 = 3;
/// The beach boulders, wet and dark: base albedo dark grey-green to black,
/// (40-70, 45-70, 40-60) (estimate; [P:baker-sunset] (35,24,15) in the surf
/// and (61,39,15) in the foreground under the sunset): its middle.
pub const SERPENTINITE_BOULDER_SRGB: [u8; 3] = [55, 60, 52];
/// The tip word that asks the props shader for serpentinite's scaly fabric.
pub const TIP_SCALY: u8 = 253;
/// The bluff knobs' and the beach boulders' size range (real m): rounded to
/// sub-rounded, mostly 0.5-3 m, a few 4-5 m (guide 4.3, [P:baker-sunset]).
pub const BOULDER_M: (f64, f64) = (0.5, 3.0);
pub const BOULDER_BIG_M: f64 = 4.5;
/// The share of boulders that are the big ones (INFERRED from "a few").
pub const BOULDER_BIG_SHARE: f64 = 0.04;
/// The Baker and Marshall's shore the boulders scatter on (lon west, lat
/// south, lon east, lat north; degrees): the serpentinite unit's box
/// (rocks::place `unit_at`) from Baker Beach's north half to the bridge.
pub const BOULDER_BOX: [f64; 4] = [-122.4860, 37.7930, -122.4740, 37.8085];
/// The band across the shore they scatter in (drawn m of the shore field's
/// signed distance, positive over the sea): the surf zone to the bluff foot.
pub const BOULDER_BAND_M: (f64, f64) = (-30.0, 18.0);

// ------------------------------------------------------------------ where the scrub grows

/// How the ground's shape weights the scrub (a factor on the third pass's
/// scrub share): the coastal scrub follows the concave, moister, sheltered
/// ground — gullies, the lee (east and north-east) sides of spurs and the
/// lower and middle slopes — while ridge crests and the windiest convex
/// shoulders carry grassland (field guide 1.1: "Where it grows" and
/// "Exposure", UNVERIFIED reading of the photographs; 1.2: grassland on the
/// ridge tops and convex upper slopes).
pub fn terrain_weight(s: &Site) -> f64 {
    let slope = s.slope_deg();
    let gully = smoothstep(0.3, 2.0, s.hollow_m);
    // a crest: convex and gentle (a steep face below a brow is not a ridge top).
    let ridge = smoothstep(0.6, 2.8, -s.hollow_m) * (1.0 - smoothstep(18.0, 30.0, slope));
    let h = (s.normal[0] * s.normal[0] + s.normal[2] * s.normal[2]).sqrt().max(1e-6);
    // world x points east, z south: an east face's normal has +x, a north face's -z.
    let east = s.normal[0] / h;
    let lee = smoothstep(0.15, 0.7, 0.7 * east + 0.7 * s.north()) * smoothstep(6.0, 14.0, slope);
    // lower and middle slopes (the headland's south face runs from about 180 game m at
    // Battery Spencer to the cliffs; game heights are 1.25 x real).
    let lower = 1.0 - smoothstep(110.0, 190.0, s.y);
    ((0.6 + 0.65 * gully + 0.35 * lee + 0.25 * lower) * (1.0 - 0.65 * ridge)).clamp(0.0, 1.4)
}

/// The species at a scrub cell and its height (game m): coyote brush
/// dominant, then sagebrush (drier south faces and shoulders) and poison oak
/// (gullies), with monkeyflower (rocky and cut slopes), coffeeberry (moister
/// pockets) and toyon (scrub) in places [S1]; the lupine where the third pass
/// put it (in flower in the bloom window). Each species grows in stands (a
/// warped field each). Shares INFERRED from the guide's order.
#[allow(clippy::too_many_arguments)]
pub fn pick(x: f64, z: f64, ix: i64, iz: i64, s: &Site, lupine: bool) -> (u8, f32) {
    let slope = s.slope_deg();
    let gully = smoothstep(0.3, 2.0, s.hollow_m);
    let north = s.north();
    let south = (-north).max(0.0);
    let stand = |cell: f64, salt: u32| smoothstep(0.4, 0.72, warped(x, z, cell, salt));
    let w = [
        (MEADOW_COYOTE, 1.0),
        (MEADOW_SAGEBRUSH, 0.9 * (0.35 + south) * (1.0 - 0.7 * gully) * stand(14.0, 131)),
        (MEADOW_POISON_OAK, 1.1 * (0.25 + gully) * stand(10.0, 132)),
        (MEADOW_MONKEYFLOWER, 0.9 * smoothstep(18.0, 38.0, slope) * stand(8.0, 133)),
        (MEADOW_COFFEEBERRY, 0.8 * (0.15 + gully + 0.5 * north.max(0.0)) * stand(12.0, 134)),
        (MEADOW_TOYON, 0.35 * (0.2 + gully) * stand(16.0, 135)),
    ];
    let kind = if lupine {
        MEADOW_LUPINE
    } else {
        let sum: f64 = w.iter().map(|v| v.1).sum();
        let mut r = hash(ix, iz, 136) * sum;
        let mut k = MEADOW_COYOTE;
        for (kind, v) in w {
            if r < v {
                k = kind;
                break;
            }
            r -= v;
        }
        k
    };
    let sp = species(kind).expect("a scrub species");
    // taller in the sheltered gullies and where the scrub is older (a 20 m field).
    let t = (0.25 + 0.4 * field(x, z, 20.0, 118) + 0.25 * gully + 0.35 * hash(ix, iz, 119)).clamp(0.0, 1.0) as f32;
    (kind, sp.height.0 + (sp.height.1 - sp.height.0) * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vegetation::Lithology;

    fn site(normal: [f64; 3], hollow_m: f64, y: f64) -> Site {
        let l = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        Site { y, normal: normal.map(|v| v / l), ground: [0.1; 3], cover: None, lithology: Lithology::Chert, shore_m: 300.0, hollow_m }
    }

    #[test]
    fn flora_rolls_up_and_reads_each_switch() {
        let a = |v: &[&str]| Flora::from_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&[]), Flora::ALL);
        assert_eq!(a(&["--no-flora"]), Flora::NONE);
        assert_eq!(a(&["--no-scrub-species"]), Flora { species: false, ..Flora::ALL });
        assert_eq!(a(&["--no-grass-ramp", "--no-bluff-scrub"]), Flora { ramp: false, bluff: false, ..Flora::ALL });
        assert_eq!(a(&["--no-chert-ground"]), Flora { chert: false, ..Flora::ALL });
        assert_eq!(a(&["--no-serpentinite"]), Flora { serpentinite: false, ..Flora::ALL });
    }

    #[test]
    fn scrub_keeps_to_gullies_and_lee_sides_and_off_the_crests() {
        let gully = terrain_weight(&site([0.0, 1.0, 0.3], 2.5, 120.0));
        let crest = terrain_weight(&site([0.0, 1.0, 0.1], -3.0, 200.0));
        let lee = terrain_weight(&site([0.25, 1.0, -0.25], 0.0, 150.0));
        let windward = terrain_weight(&site([-0.25, 1.0, 0.25], 0.0, 150.0));
        assert!(gully > 1.0 && crest < 0.3, "gully {gully} crest {crest}");
        assert!(lee > windward, "lee {lee} windward {windward}");
    }

    #[test]
    fn coyote_brush_dominates_and_every_species_grows() {
        let s = site([0.0, 1.0, 0.4], 0.8, 120.0);
        let mut n = std::collections::HashMap::new();
        for ix in 0..120 {
            for iz in 0..120 {
                *n.entry(pick(ix as f64 * 1.0, iz as f64 * 1.0, ix, iz, &s, false).0).or_insert(0) += 1;
            }
        }
        let coyote = n[&MEADOW_COYOTE];
        for sp in SPECIES.iter().filter(|s| s.kind != MEADOW_LUPINE) {
            assert!(n.get(&sp.kind).copied().unwrap_or(0) > 0, "{} missing", sp.name);
            assert!(sp.kind == MEADOW_COYOTE || n[&sp.kind] < coyote, "{} outnumbers coyote brush", sp.name);
        }
    }

    #[test]
    fn the_grassland_follows_the_guides_months() {
        // green Dec-Mar, starting in April, straw by July, bleached in October.
        for d in [1.0, 40.0, 70.0, 350.0] {
            assert!(grass_cure(d) < 0.12, "day {d}: {}", grass_cure(d));
        }
        assert!(grass_cure(103.0) > 0.05 && grass_cure(103.0) < 0.2);
        assert!(grass_cure(200.0) > 0.85 && grass_cure(280.0) > 0.95);
        assert!(grass_bleach(285.0) > 0.9 && grass_bleach(103.0) == 0.0);
        assert!(seed_stalks(103.0) > 0.15 && seed_stalks(10.0) == 0.0);
        // the curve is continuous across the year's end.
        assert!((grass_cure(364.9) - grass_cure(0.0)).abs() < 0.02);
    }

    #[test]
    fn the_chert_colours_are_the_measured_ones() {
        assert_eq!(CHERT_TOP_SRGB, [190, 129, 75]);
        assert_eq!(CHERT_FACE_SRGB, [154, 120, 74]);
        // red over green over blue, the face darker than the top.
        assert!(CHERT_FACE_SRGB.iter().zip(CHERT_TOP_SRGB.iter()).all(|(f, t)| f <= t));
    }

    #[test]
    fn poison_oak_reddens_in_autumn() {
        let sep = poison_oak_season(275.0);
        let apr = poison_oak_season(103.0);
        assert!(sep[0] > 2.5 && sep[1] < 0.4);
        assert!(apr.iter().all(|v| (v - 1.0).abs() < 1e-3));
    }
}
