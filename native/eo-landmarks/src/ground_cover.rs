//! the ground of the open spaces at the terrain's vertices: the
//! mapped greens' surface tones and the beach sand, as the JS streamed tiles
//! colour their ground.
//!
//! | Rust | JS |
//! | --- | --- |
//! | `surface_tone` | src/world/green-surface-tone.js `greenSurfaceTone`, `SURVEYED_SURFACE_TONES` |
//! | `GroundCover::tone_at` | src/world/stream/tile-ground-cover.js: greens painted parks first, then larger before smaller (a lawn, court or garden mapped inside a park draws over it), the fallback court / play surface / paved tone, playground beds (`playBedField`), inland water; `DERIVED_TONES` |
//! | `ground_mottle` | src/world/stream/city-tile-derive.js `groundMottle` (48 m value noise, `lattice` hash) mixing a tone pair |
//! | `GroundCover::beach_sand` | src/world/terrain/beach-sand.js `beachSandSampler` (inside a mapped beach, or within 150 m of one and 90 m of the sea), city-tile-derive.js `GROUND_SAND` / `GROUND_SAND_DARK` |
//!
//! The JS rasterises the tones on a 2 m grid per streamed tile; here they are
//! the terrain's 8 m vertex colours (a green narrower than a cell can miss
//! its vertices; the street surfaces draw over most park edges). Paved greens
//! keep the terrain's own colour (the JS draws them as lot ground or as plaza
//! paving, which the plaza port lays).

use crate::plaza::Area;
use crate::shore::BeachFeature;
use anyhow::Result;
use std::collections::HashMap;

pub type Linear = [f64; 3];

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn hex(h: &str) -> Linear {
    let n = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
    [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(|v| srgb_to_linear(v as f64 / 255.0))
}

/// The tone pairs (green-surface-tone.js `SURVEYED_SURFACE_TONES`, tile-ground-cover.js `DERIVED_TONES`).
fn pair(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "lotAsphalt" => ("#565c64", "#636971"),
        "grass" => ("#56763f", "#66814b"),
        "understory" => ("#46553a", "#556043"),
        "sand" => ("#978b72", "#8c8169"),
        "dryGrass" => ("#7a7868", "#878473"),
        "wornDirt" => ("#8f8672", "#9b927d"),
        "duneScrub" => ("#5d6649", "#958a70"),
        "bareEarth" => ("#7a7262", "#887f6d"),
        "bareRock" => ("#7b7a73", "#8b8981"),
        "reservoirLid" => ("#86857f", "#93918a"),
        "court" => ("#5f6a62", "#6a7369"),
        "playSurface" => ("#8a8473", "#9c9584"),
        "water" => ("#1f3036", "#2a3d3f"),
        "wildGrass" => ("#5f6a40", "#7a7656"),
        "wildScrub" => ("#4e5639", "#646548"),
        "yardHard" => ("#6a675f", "#78756c"),
        _ => return None,
    })
}

/// the ground beyond the city's rings (the Marin headlands) by its
/// WorldCover class, in tile-ground-cover.js's tones for unmapped open ground
/// (`wildGrass`, `wildScrub`), under trees `understory`, bare `bareEarth`,
/// built `yardHard`; mixed by the ground mottle. Linear.
pub fn far_land_tone(class: u8, x: f64, z: f64) -> Linear {
    let name = match class {
        10 => "understory",
        30 | 40 | 100 => "wildGrass",
        50 => "yardHard",
        60 => "bareEarth",
        _ => "wildScrub",
    };
    let (a, b) = pair(name).unwrap_or(("#4e5639", "#646548"));
    let (a, b) = (hex(a), hex(b));
    let t = ground_mottle(x, z);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// A named tone pair (`pair`) mixed by the ground mottle at (x, z), linear; None for an unknown name.
pub fn named_tone(name: &str, x: f64, z: f64) -> Option<Linear> {
    let (a, b) = pair(name)?;
    let (a, b) = (hex(a), hex(b));
    let t = ground_mottle(x, z);
    Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t])
}

/// `greenSurfaceTone(g)`: the surveyed surface a green's own ground takes, or None.
pub fn surface_tone(g: &Area) -> Option<&'static str> {
    match g.ground.as_str() {
        "sand" | "bunker" => return Some("sand"),
        "bare" => return Some("bareEarth"),
        "rock" => return Some("bareRock"),
        "lid" => return Some("reservoirLid"),
        "vegetated" => {}
        _ => return None,
    }
    Some(match g.kind.as_str() {
        "park" if g.id == "osm-green-w609650218" => "duneScrub",
        "park" => "grass",
        "dog_park" => {
            if g.surface == "grass" {
                "grass"
            } else {
                "dryGrass"
            }
        }
        "vacant_lot" => "dryGrass",
        "playground" | "pitch" => "grass",
        "wood" | "forest" | "scrub" | "heath" | "nature_reserve" | "wetland" => "understory",
        _ => "grass",
    })
}

/// tile-ground-cover.js's tone id of a green: its surface tone, else sand /
/// paved / play surface / court. None for paved (the tile's own ground shows).
fn tone_of(g: &Area) -> Option<&'static str> {
    if let Some(t) = surface_tone(g) {
        return Some(t);
    }
    if g.ground == "sand" {
        Some("sand")
    } else if g.ground == "paved" {
        None
    } else if g.kind == "playground" {
        Some("playSurface")
    } else {
        Some("court")
    }
}

/// tile-ground-cover.js `hash2` / city-tile-derive.js `lattice`: [0, 1].
pub fn hash2(a: i64, b: i64) -> f64 {
    let h = (a as i32).wrapping_mul(374_761_393).wrapping_add((b as i32).wrapping_mul(668_265_263));
    let h = (h ^ ((h as u32 >> 13) as i32)).wrapping_mul(1_274_126_177);
    ((h ^ ((h as u32 >> 16) as i32)) as u32) as f64 / 4_294_967_295.0
}

fn value_noise(x: f64, z: f64, s: f64, off: i64) -> f64 {
    let (gx, gz) = (x / s, z / s);
    let (ix, iz) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - ix, gz - iz);
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let (ix, iz) = (ix as i64, iz as i64);
    let a = hash2(ix, iz + off);
    let b = hash2(ix + 1, iz + off);
    let c = hash2(ix, iz + off + 1);
    let d = hash2(ix + 1, iz + off + 1);
    (a + (b - a) * ux) * (1.0 - uz) + (c + (d - c) * ux) * uz
}

/// `groundMottle`: smooth value noise in [0, 1] at 48 m.
pub fn ground_mottle(x: f64, z: f64) -> f64 {
    value_noise(x, z, 48.0, 0)
}

/// `playBedField`: the playground's beds about 10 m across.
pub fn play_bed_field(x: f64, z: f64) -> f64 {
    value_noise(x, z, 9.0, 7717)
}

/// One park tree (tile-trees.js `plantTileTrees`'s record without its tint).
#[derive(Clone, Copy, Debug)]
pub struct ParkTree {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub scale: f64,
    pub yaw: f64,
    /// The ez-tree species (`park-pine`, `park-oak`, `park-ash`).
    pub species: &'static str,
}

struct TreeRule {
    chance: f64,
    grove: f64,
    scale: [f64; 2],
    species: &'static [&'static str],
}

const BIG: &[&str] = &["park-pine", "park-oak", "park-ash"];
const GARDEN: &[&str] = &["park-ash", "park-oak"];

/// tile-trees.js `RULES` for the mapped classes (the derived yard classes are not ported).
fn tree_rule(class: u8) -> Option<TreeRule> {
    Some(match class {
        1 => TreeRule { chance: 0.86, grove: 0.2, scale: [1.3, 2.2], species: BIG },
        2 => TreeRule { chance: 0.62, grove: 1.0, scale: [1.1, 2.0], species: BIG },
        3 => TreeRule { chance: 0.28, grove: 1.0, scale: [0.8, 1.3], species: BIG },
        4 => TreeRule { chance: 0.3, grove: 0.5, scale: [0.6, 1.1], species: BIG },
        5 => TreeRule { chance: 0.26, grove: 0.4, scale: [0.45, 0.85], species: GARDEN },
        6 => TreeRule { chance: 0.14, grove: 0.8, scale: [0.35, 0.6], species: BIG },
        7 => TreeRule { chance: 0.04, grove: 0.6, scale: [0.5, 0.9], species: GARDEN },
        _ => return None,
    })
}

/// tile-trees.js `hash2(a, b, salt)`.
pub fn tree_hash(a: i64, b: i64, salt: i64) -> f64 {
    let h = (a as i32).wrapping_mul(73_856_093) ^ (b as i32).wrapping_mul(19_349_663) ^ (salt as i32).wrapping_mul(83_492_791);
    let h = (h ^ ((h as u32 >> 13) as i32)).wrapping_mul(1_274_126_177);
    ((h ^ ((h as u32 >> 16) as i32)) as u32) as f64 / 4_294_967_295.0
}

/// tile-trees.js `groveField`: the ~70 m grove field.
pub fn grove_field(x: f64, z: f64) -> f64 {
    let s = 70.0;
    let (gx, gz) = (x / s, z / s);
    let (ixf, izf) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - ixf, gz - izf);
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let (ix, iz) = (ixf as i64, izf as i64);
    let (a, b) = (tree_hash(ix, iz, 11), tree_hash(ix + 1, iz, 11));
    let (c, d) = (tree_hash(ix, iz + 1, 11), tree_hash(ix + 1, iz + 1, 11));
    let v = (a + (b - a) * ux) * (1.0 - uz) + (c + (d - c) * ux) * uz;
    let t = ((v - 0.4) / 0.25).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// the ground cover the props tiles plant park trees from (set
/// once at load; none with `--no-landmarks2`).
static INSTALLED: std::sync::OnceLock<std::sync::Arc<GroundCover>> = std::sync::OnceLock::new();

pub fn install(c: std::sync::Arc<GroundCover>) {
    let _ = INSTALLED.set(c);
}

pub fn installed() -> Option<&'static std::sync::Arc<GroundCover>> {
    INSTALLED.get()
}

/// city-tile-derive.js `GROUND_SAND` / `GROUND_SAND_DARK` (linear).
pub const GROUND_SAND: Linear = [0.366, 0.337, 0.275];
pub const GROUND_SAND_DARK: Linear = [0.305, 0.275, 0.216];
/// The dry beach sand's broadband albedo (`--no-sand-albedo`: the JS tones
/// above, whose mean luminance is 0.31): Ångström's pyranometer measurement
/// of dry sand, 0.182 (wet: 0.091), as tabulated in J. Lekner and M. C. Dorf,
/// "Why some things are darker when wet", Applied Optics 27, 1278 (1988).
/// The JS pair keeps its hue and mottle and is scaled to this mean luminance.
pub const DRY_SAND_ALBEDO: f64 = 0.182;
/// Wet over dry sand albedo, the same measurement: 0.091 / 0.182.
pub const WET_SAND_RATIO: f64 = 0.5;

fn luma(c: Linear) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The factor that takes the JS sand pair to `DRY_SAND_ALBEDO` (the mean of
/// its two tones' luminance, as the mottle mixes them half and half).
pub fn sand_albedo_scale() -> f64 {
    DRY_SAND_ALBEDO / (0.5 * (luma(GROUND_SAND) + luma(GROUND_SAND_DARK)))
}

/// beach-sand.js: how far past a mapped beach the sand runs, and how near the sea it must be.
pub const BEACH_SAND_REACH_M: f64 = 150.0;
pub const BEACH_SAND_SHORE_M: f64 = 90.0;

struct Green {
    ring: Vec<[f64; 2]>,
    holes: Vec<Vec<[f64; 2]>>,
    min: [f64; 2],
    max: [f64; 2],
    /// Paint order: parks first, then larger boxes before smaller; the last painter wins.
    order: (u8, f64),
    tone: Option<&'static str>,
    beds: bool,
    /// tile-ground-cover.js `treeClassOf` (0 none), for the park trees.
    tree_class: u8,
}

fn in_ring(r: &[[f64; 2]], x: f64, z: f64) -> bool {
    crate::waterfront::point_in_ring(r, x, z)
}

/// The greens, inland water and beaches, indexed on a coarse grid.
pub struct GroundCover {
    greens: Vec<Green>,
    cells: HashMap<(i64, i64), Vec<u32>>,
    beaches: Vec<(Vec<[f64; 2]>, [f64; 2], [f64; 2])>,
    beach_cells: HashMap<(i64, i64), Vec<u32>>,
}

const CELL: f64 = 64.0;

/// tile-ground-cover.js `TREE_CLASS` by green kind (`treeClassOf`).
pub fn tree_class_of(kind: &str) -> u8 {
    match kind {
        "wood" | "forest" => 1,
        "park" | "common" | "recreation_ground" | "village_green" | "nature_reserve" | "zoo" => 2,
        "golf_course" => 3,
        "cemetery" => 4,
        "garden" | "allotments" => 5,
        "scrub" | "heath" | "wetland" => 6,
        "grass" | "meadow" | "grassland" => 7,
        _ => 0,
    }
}

impl GroundCover {
    pub fn load(repo_root: &std::path::Path) -> Result<Self> {
        let (greens, water) = crate::plaza::load_areas(repo_root)?;
        let beaches = crate::shore::load_beaches(repo_root)?;
        Ok(Self::new(&greens, &water, &beaches))
    }

    pub fn new(greens: &[Area], water: &[Area], beaches: &[BeachFeature]) -> Self {
        let bare = ["sand", "duneScrub", "bareEarth", "bareRock", "reservoirLid"];
        let mut list = Vec::new();
        for g in greens {
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &g.polygon {
                for k in 0..2 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            let tone = tone_of(g);
            let surface = surface_tone(g);
            let tree_class = if surface.is_some_and(|s| !bare.contains(&s)) && g.kind != "pitch" && g.kind != "playground" { tree_class_of(&g.kind) } else { 0 };
            list.push(Green {
                ring: g.polygon.clone(),
                holes: g.holes.clone(),
                min: mn,
                max: mx,
                order: (if g.kind == "park" { 0 } else { 1 }, -((mx[0] - mn[0]) * (mx[1] - mn[1]))),
                tone,
                beds: g.kind == "playground" && tone == Some("playSurface"),
                tree_class,
            });
        }
        // Inland standing water, painted after every green (no tree stands in it).
        for w in water.iter().filter(|w| w.kind != "beach") {
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &w.polygon {
                for k in 0..2 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            list.push(Green { ring: w.polygon.clone(), holes: w.holes.clone(), min: mn, max: mx, order: (2, 0.0), tone: Some("water"), beds: false, tree_class: 0 });
        }
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, g) in list.iter().enumerate() {
            for r in (g.min[1] / CELL).floor() as i64..=(g.max[1] / CELL).floor() as i64 {
                for c in (g.min[0] / CELL).floor() as i64..=(g.max[0] / CELL).floor() as i64 {
                    cells.entry((c, r)).or_default().push(i as u32);
                }
            }
        }
        let mut bl = Vec::new();
        let mut beach_cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, b) in beaches.iter().enumerate() {
            let (mut mn, mut mx) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &b.polygon {
                for k in 0..2 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            let pad = BEACH_SAND_REACH_M;
            for r in ((mn[1] - pad) / CELL).floor() as i64..=((mx[1] + pad) / CELL).floor() as i64 {
                for c in ((mn[0] - pad) / CELL).floor() as i64..=((mx[0] + pad) / CELL).floor() as i64 {
                    beach_cells.entry((c, r)).or_default().push(i as u32);
                }
            }
            bl.push((b.polygon.clone(), mn, mx));
        }
        Self { greens: list, cells, beaches: bl, beach_cells }
    }

    /// The green (or inland water) painted last at (x, z), by the JS order.
    fn top(&self, x: f64, z: f64) -> Option<&Green> {
        let list = self.cells.get(&((x / CELL).floor() as i64, (z / CELL).floor() as i64))?;
        let mut best: Option<&Green> = None;
        for &i in list {
            let g = &self.greens[i as usize];
            if x < g.min[0] || x > g.max[0] || z < g.min[1] || z > g.max[1] {
                continue;
            }
            if best.is_some_and(|b| (g.order.0, g.order.1) <= (b.order.0, b.order.1)) {
                continue;
            }
            if in_ring(&g.ring, x, z) && !g.holes.iter().any(|h| h.len() >= 3 && in_ring(h, x, z)) {
                best = Some(g);
            }
        }
        best
    }

    /// The ground tone at (x, z), linear, or None (no green, or a paved one).
    pub fn tone_at(&self, x: f64, z: f64) -> Option<Linear> {
        let g = self.top(x, z)?;
        let name = if g.beds && play_bed_field(x, z) < 0.5 { "grass" } else { g.tone? };
        let (a, b) = pair(name)?;
        let (a, b) = (hex(a), hex(b));
        let t = ground_mottle(x, z);
        Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t])
    }

    /// the tone name `tone_at` draws at (x, z) (`grass`, `understory`, `court`, ...), or None.
    pub fn tone_name_at(&self, x: f64, z: f64) -> Option<&'static str> {
        let g = self.top(x, z)?;
        if g.beds && play_bed_field(x, z) < 0.5 { Some("grass") } else { g.tone }
    }

    /// The park-tree class at (x, z) (`treeClassOf` of the green painted there; 0 none).
    pub fn tree_class_at(&self, x: f64, z: f64) -> u8 {
        self.top(x, z).map(|g| g.tree_class).unwrap_or(0)
    }

    /// `beachSandSampler`: inside a mapped beach, or within `BEACH_SAND_REACH_M`
    /// of one and `BEACH_SAND_SHORE_M` of the sea (`shore_m`: distance to the shoreline).
    pub fn beach_sand(&self, x: f64, z: f64, shore_m: f64) -> bool {
        let Some(list) = self.beach_cells.get(&((x / CELL).floor() as i64, (z / CELL).floor() as i64)) else { return false };
        let near_sea = shore_m <= BEACH_SAND_SHORE_M;
        for &i in list {
            let (ring, mn, mx) = &self.beaches[i as usize];
            if x >= mn[0] && x <= mx[0] && z >= mn[1] && z <= mx[1] && in_ring(ring, x, z) {
                return true;
            }
            if near_sea && x >= mn[0] - BEACH_SAND_REACH_M && x <= mx[0] + BEACH_SAND_REACH_M && z >= mn[1] - BEACH_SAND_REACH_M && z <= mx[1] + BEACH_SAND_REACH_M && crate::waterfront::ring_distance(ring, x, z) <= BEACH_SAND_REACH_M {
                return true;
            }
        }
        false
    }

    /// Whether (x, z) lies inside a mapped beach's ring (OSM `natural=beach`),
    /// not merely within `BEACH_SAND_REACH_M` of one.
    pub fn in_mapped_beach(&self, x: f64, z: f64) -> bool {
        let Some(list) = self.beach_cells.get(&((x / CELL).floor() as i64, (z / CELL).floor() as i64)) else { return false };
        list.iter().any(|&i| {
            let (ring, mn, mx) = &self.beaches[i as usize];
            x >= mn[0] && x <= mx[0] && z >= mn[1] && z <= mx[1] && in_ring(ring, x, z)
        })
    }

    /// Whether (x, z) lies within `BEACH_SAND_REACH_M` of a mapped beach (at
    /// any distance from the sea): the cliffs and bluffs over the beaches.
    pub fn near_mapped_beach(&self, x: f64, z: f64) -> bool {
        let Some(list) = self.beach_cells.get(&((x / CELL).floor() as i64, (z / CELL).floor() as i64)) else { return false };
        list.iter().any(|&i| {
            let (ring, mn, mx) = &self.beaches[i as usize];
            let r = BEACH_SAND_REACH_M;
            x >= mn[0] - r && x <= mx[0] + r && z >= mn[1] - r && z <= mx[1] + r && (in_ring(ring, x, z) || crate::waterfront::ring_distance(ring, x, z) <= r)
        })
    }

    /// tile-trees.js `plantTileTrees` over [x0, x1) x [z0, z1) (near tier): the
    /// world-anchored 14 m lattice jittered by `hash2`, a tree where the green
    /// painted there has a tree class, its rule's chance (times the grove
    /// field) passes and nothing within one 2 m cell (`blocked`: a footprint,
    /// a drawn street surface, water) is in the way; at most `maxNear`, thinned
    /// by hash. `y_at` is the drawn ground.
    pub fn park_trees(&self, x0: f64, z0: f64, x1: f64, z1: f64, y_at: &dyn Fn(f64, f64) -> f64, blocked: &dyn Fn(f64, f64) -> bool) -> Vec<ParkTree> {
        const S: f64 = 14.0;
        const CELL_M: f64 = 2.0;
        const MAX_NEAR: usize = 150;
        const ROOT_SINK_M: f64 = 0.08;
        let mut out: Vec<(ParkTree, f64)> = Vec::new();
        for iz in (z0 / S).floor() as i64 - 1..=(z1 / S).ceil() as i64 {
            for ix in (x0 / S).floor() as i64 - 1..=(x1 / S).ceil() as i64 {
                let x = (ix as f64 + 0.15 + tree_hash(ix, iz, 1) * 0.7) * S;
                let z = (iz as f64 + 0.15 + tree_hash(ix, iz, 2) * 0.7) * S;
                if x < x0 || x >= x1 || z < z0 || z >= z1 {
                    continue;
                }
                let Some(rule) = tree_rule(self.tree_class_at(x, z)) else { continue };
                let mut clear = true;
                'k: for dz in [-1.0, 0.0, 1.0] {
                    for dx in [-1.0, 0.0, 1.0] {
                        if blocked(x + dx * CELL_M, z + dz * CELL_M) {
                            clear = false;
                            break 'k;
                        }
                    }
                }
                if !clear {
                    continue;
                }
                let roll = tree_hash(ix, iz, 3);
                let grove = if rule.grove > 0.0 { (1.0 - rule.grove) + rule.grove * grove_field(x, z) } else { 1.0 };
                if roll >= rule.chance * grove {
                    continue;
                }
                let y = y_at(x, z);
                if !y.is_finite() {
                    continue;
                }
                let n = rule.species.len();
                let pick = ((tree_hash(ix, iz, 4) * n as f64).floor() as usize) % n;
                let scale = rule.scale[0] + tree_hash(ix, iz, 5) * (rule.scale[1] - rule.scale[0]);
                out.push((ParkTree { x, y: y - ROOT_SINK_M, z, scale, yaw: tree_hash(ix, iz, 6) * std::f64::consts::TAU, species: rule.species[pick] }, tree_hash(ix, iz, 8)));
            }
        }
        if out.len() > MAX_NEAR {
            out.sort_by(|a, b| a.1.total_cmp(&b.1));
            out.truncate(MAX_NEAR);
        }
        out.into_iter().map(|(t, _)| t).collect()
    }

    /// The beach sand's colour at (x, z): `GROUND_SAND` to `GROUND_SAND_DARK`
    /// by the mottle, scaled to the measured dry-sand albedo (`sand_albedo_scale`)
    /// unless `--no-sand-albedo`.
    pub fn sand_at(&self, x: f64, z: f64) -> Linear {
        sand_tone(x, z, crate::features::shore_ground("sand-albedo"))
    }
}

/// `GroundCover::sand_at` with the albedo switch given.
pub fn sand_tone(x: f64, z: f64, measured: bool) -> Linear {
    let t = ground_mottle(x, z);
    let k = if measured { sand_albedo_scale() } else { 1.0 };
    [0, 1, 2].map(|i| (GROUND_SAND[i] + (GROUND_SAND_DARK[i] - GROUND_SAND[i]) * t) * k)
}

#[cfg(test)]
mod sand_tests {
    use super::*;

    #[test]
    fn the_sand_has_the_measured_albedo() {
        // mean over the mottle: the measured dry albedo; the JS tones without the switch
        let mean = |on: bool| (0..400).map(|i| luma(sand_tone(i as f64 * 7.3, i as f64 * 3.1, on))).sum::<f64>() / 400.0;
        assert!((mean(true) - DRY_SAND_ALBEDO).abs() < 0.015, "{}", mean(true));
        assert!((mean(false) - 0.31).abs() < 0.02, "{}", mean(false));
        assert_eq!(sand_tone(5.0, 9.0, false)[0], GROUND_SAND[0] + (GROUND_SAND_DARK[0] - GROUND_SAND[0]) * ground_mottle(5.0, 9.0));
        // the hue is the JS pair's
        let (a, b) = (sand_tone(5.0, 9.0, true), sand_tone(5.0, 9.0, false));
        assert!((a[0] / a[2] - b[0] / b[2]).abs() < 1e-9);
    }
}
