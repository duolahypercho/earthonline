//! Rear yards measured from the parcel map, the lidar and the aerial
//! (`--yard-detail`, off by default; `--no-yard-detail`). The yard survey
//! (eo-kit `examples/yard_survey.rs`) writes into `$EO_PARCELS_DIR/<area>/` (a
//! folder outside the repository; the parcels come from DataSF, PDDL 1.0,
//! through `native/tools/fetch-parcels.py`):
//!
//! * `yards.json`: the lot lines two parcels share and, on each, the spans
//!   where the USGS 3DEP lidar finds a fence, with the fence's measured height;
//! * `yard-ground.bin.gz`: the yard ground class of every 0.5 m cell (lawn,
//!   planting, paving, deck) from the USDA NAIP aerial and the lidar;
//! * `yard-canopy.bin.gz` (optional): every yard cell's vegetation height from
//!   the lidar and its colour from the aerial. With it the yards' measured
//!   vegetation stands as plants (`plants`): trees where the canopy is tall and
//!   no measured tree (`lidar_trees`) already covers it, shrubs under that, and
//!   small plants in the beds, instead of only the ground's planting tone.
//!
//! Both are on the surveyed 1:1 plan; only positions follow the ground plan's
//! scale (`eo_data::groundscale`), heights stay real. Every area found there is
//! read once. The fences are drawn as props (eo-app `yards.rs`, eo-render
//! `props_yards.rs`), the ground classes by the terrain's shader (eo-render
//! `yards.rs`).

use std::path::PathBuf;
use std::sync::OnceLock;

/// `--yard-detail` sets `EO_YARD_DETAIL=on`, `--no-yard-detail` sets it `off`.
pub fn enabled() -> bool {
    std::env::var("EO_YARD_DETAIL").is_ok_and(|v| v == "on" || v == "1")
}

/// Under `--yard-detail`, the canopy map read (`yard-canopy.bin.gz`): the yards'
/// plants and the ground's aerial tones (`plants`, eo-render yards.rs);
/// `--no-yard-plants` (`EO_YARD_PLANTS=off`) leaves the map unread and restores
/// the yards as they were before it.
pub fn plants_enabled() -> bool {
    enabled() && std::env::var("EO_YARD_PLANTS").map_or(true, |v| v != "off")
}

/// With the plants, the yards' objects (`items`: collection carts, patio sets,
/// grills, pots, raised beds, sheds); `--no-yard-objects`
/// (`EO_YARD_OBJECTS=off`) leaves them out.
pub fn items_enabled() -> bool {
    plants_enabled() && std::env::var("EO_YARD_OBJECTS").map_or(true, |v| v != "off")
}

/// With the plants, the backs of the yards' houses (eo-app `yards/backs.rs`: wooden
/// back stairs with their landings and doors, from the kit's plans, and downspouts);
/// `--no-yard-stairs` (`EO_YARD_STAIRS=off`) leaves them out.
pub fn backs_enabled() -> bool {
    plants_enabled() && std::env::var("EO_YARD_STAIRS").map_or(true, |v| v != "off")
}

/// With the plants, the wood of the yards' objects and stairs (eo-render props_yards.wgsl
/// `pr_yard_item`: the grain, paint worn through to grey wood); `--no-yard-wood`
/// (`EO_YARD_WOOD=off`) draws them in their flat colours as before.
pub fn wood_enabled() -> bool {
    plants_enabled() && std::env::var("EO_YARD_WOOD").map_or(true, |v| v != "off")
}

/// The ground classes of `yard-ground.bin.gz` (0: not a yard).
pub const LAWN: u8 = 1;
pub const PLANTING: u8 = 2;
pub const PAVING: u8 = 3;
pub const DECK: u8 = 4;
pub const CLASSES: u8 = 5;
/// `yard-ground.bin.gz`'s first bytes (then nx, nz as u32, x0, z0, cell as f32, a byte a cell).
pub const GROUND_MAGIC: &[u8; 8] = b"EOYARDG1";
/// `yard-canopy.bin.gz`'s first bytes (the same header, then five bytes a cell:
/// the vegetation height over the ground in 0.1 m, the aerial's sRGB8 colour,
/// the class with the paving laid square, which replaces the ground file's).
pub const CANOPY_MAGIC: &[u8; 8] = b"EOYARDC2";

/// The canopy's plants (`plants`). A canopy cell taller than this with no
/// measured tree over it grows a tree (m): the lidar survey's own tree floor
/// (eo-kit `lidar_survey`, `MIN_TREE_M`).
pub const TREE_MIN_M: f64 = 3.0;
/// Lower vegetation is a shrub (m): the survey's vegetation floor (`VEG_MIN_M`).
pub const SHRUB_MIN_M: f64 = 0.5;
/// The tallest shrub (m): a cell over it that no tree took is the edge of a crown.
pub const SHRUB_MAX_M: f64 = 3.5;
/// A crown of the canopy: the cells grown downhill from its top down to this
/// share of the top's height (the lidar survey's `CROWN_BASE_FRACTION` is 0.3; a
/// little higher here, since what is left after the measured trees is mostly
/// merged canopy) and at least `CROWN_MIN_M2` (drawn m^2).
pub const CROWN_BASE_SHARE: f64 = 0.45;
pub const CROWN_MIN_M2: f64 = 3.0;
/// The share of a measured tree's crown radius taken as covered (its crown's
/// edge is open leaves, where a neighbour may reach in).
pub const COVER_SHARE: f64 = 0.85;
/// The shrubs' grid (drawn m) and the share of a grid square the vegetation
/// must fill to hold one.
pub const SHRUB_PITCH_M: f64 = 1.9;
pub const SHRUB_FILL: f64 = 0.3;
/// The beds' small plants (the planting class at ground level): their grid
/// (drawn m), the share of squares planted, their height range (m).
pub const BED_PITCH_M: f64 = 1.4;
pub const BED_SHARE: f64 = 0.4;
pub const BED_HEIGHT_M: (f64, f64) = (0.35, 0.9);
/// The lawn's turf patches: their grid (drawn m; a patch is about 1.4 m
/// across, so neighbours overlap) and the share of a square the lawn must fill.
pub const LAWN_PITCH_M: f64 = 0.9;
pub const LAWN_FILL: f64 = 0.4;
/// Rough grass and weeds in the beds' litter under the plants: on the lawn's
/// grid, where a 3 m field puts them (over half the litter, in drifts; an
/// unkempt bed's grass and weeds, INFERRED), three to eight tufts a square by
/// the drift's strength, each `WEEDS_TUFT_M` across and `WEEDS_HEIGHT_M` tall.
/// The open beds (the planting class at ground level: the aerial's low green)
/// take them over `WEEDS_OPEN_SHARE` of their ground: groundcover, weeds and
/// self-sown grass round the bed plants.
pub const WEEDS_SHARE: f64 = 0.55;
pub const WEEDS_OPEN_SHARE: f64 = 0.8;
pub const WEEDS_HEIGHT_M: (f64, f64) = (0.08, 0.25);
pub const WEEDS_TUFT_M: (f64, f64) = (0.3, 0.8);

/// A fence's thickness (m): a 1 in board (3/4 in dressed) on 2 in rails (1-1/2 in
/// dressed), the dressed sizes of the American Softwood Lumber Standard (NIST
/// Voluntary Product Standard PS 20, dry sizes of nominal 1 and 2 in lumber).
pub const THICKNESS_M: f64 = (0.75 + 1.5) * 0.0254;
/// The fence pieces' longest run (m): each follows the drawn ground between its
/// ends (the terrain's vertices are 8 m apart; a piece is a straight shear).
pub const PIECE_MAX_M: f64 = 2.0;
/// The steps along a fence at which the drawn street and buildings are tested (m).
pub const SAMPLE_M: f64 = 0.25;
/// How far a piece's foot goes under the drawn ground (m), so a crease of the
/// terrain between its ends shows no gap; its top stays at the measured height.
pub const SINK_M: f64 = 0.1;

/// One measured fence, on the drawn (scaled) plan.
#[derive(Clone, Copy, Debug)]
pub struct Fence {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub height_m: f64,
}

/// One area's ground classes, on the drawn plan: cell (i, j) covers
/// [x0 + i cell, x0 + (i + 1) cell) x [z0 + j cell, ...).
#[derive(Clone, Debug)]
pub struct GroundMap {
    pub x0: f64,
    pub z0: f64,
    pub cell: f64,
    pub nx: u32,
    pub nz: u32,
    pub classes: Vec<u8>,
    /// `yard-canopy.bin.gz`'s cells (height in 0.1 m, sRGB8), or empty without it.
    pub canopy: Vec<[u8; 4]>,
}

impl GroundMap {
    /// Decode `yard-ground.bin.gz`'s bytes (already gunzipped) onto the drawn plan.
    pub fn decode(b: &[u8], scale: f64) -> Result<GroundMap, String> {
        if b.len() < 28 || &b[..8] != GROUND_MAGIC {
            return Err("not a yard ground map".into());
        }
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64;
        let (nx, nz) = (u(8), u(12));
        let classes = b[28..].to_vec();
        if classes.len() != nx as usize * nz as usize || classes.iter().any(|&c| c >= CLASSES) {
            return Err(format!("{} cells for {nx} x {nz}, or a class past {}", classes.len(), CLASSES - 1));
        }
        Ok(GroundMap { x0: f(16) * scale, z0: f(20) * scale, cell: f(24) * scale, nx, nz, classes, canopy: vec![] })
    }
    /// Decode `yard-canopy.bin.gz`'s bytes (already gunzipped) onto this map:
    /// its grid must be the ground's; its classes replace the ground file's.
    pub fn decode_canopy(&mut self, b: &[u8], scale: f64) -> Result<(), String> {
        if b.len() < 28 || &b[..8] != CANOPY_MAGIC {
            return Err("not a yard canopy map".into());
        }
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64;
        let same = (u(8), u(12)) == (self.nx, self.nz) && [f(16) * scale - self.x0, f(20) * scale - self.z0, f(24) * scale - self.cell].iter().all(|d| d.abs() < 1e-3);
        if !same {
            return Err("its grid is not the ground map's".into());
        }
        let cells = &b[28..];
        if cells.len() != self.classes.len() * 5 || cells.chunks_exact(5).any(|c| c[4] >= CLASSES) {
            return Err(format!("{} bytes for {} cells, or a class past {}", cells.len(), self.classes.len(), CLASSES - 1));
        }
        self.canopy = cells.chunks_exact(5).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        self.classes = cells.chunks_exact(5).map(|c| c[4]).collect();
        Ok(())
    }
    /// The vegetation height at cell `k` (m; 0 without the canopy map).
    pub fn height_m(&self, k: usize) -> f64 {
        self.canopy.get(k).map_or(0.0, |c| c[0] as f64 * 0.1)
    }
    /// The aerial's colour at cell `k` (linear), if the canopy map is read.
    pub fn colour(&self, k: usize) -> Option<[f32; 3]> {
        let c = self.canopy.get(k)?;
        Some([c[1], c[2], c[3]].map(|v| eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0)))
    }
    /// Cell `k`'s centre on the drawn plan.
    pub fn centre(&self, k: usize) -> (f64, f64) {
        let n = self.nx as usize;
        (self.x0 + ((k % n) as f64 + 0.5) * self.cell, self.z0 + ((k / n) as f64 + 0.5) * self.cell)
    }
    /// The class at a drawn point (0 outside).
    pub fn at(&self, x: f64, z: f64) -> u8 {
        let (i, j) = (((x - self.x0) / self.cell).floor(), ((z - self.z0) / self.cell).floor());
        if i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.nz as f64 {
            return 0;
        }
        self.classes[j as usize * self.nx as usize + i as usize]
    }
    /// Area of each class (drawn m^2).
    pub fn areas(&self) -> [f64; CLASSES as usize] {
        let mut a = [0.0; CLASSES as usize];
        for &c in &self.classes {
            a[c as usize] += self.cell * self.cell;
        }
        a
    }
}

struct Installed {
    fences: Vec<Fence>,
    grounds: Vec<GroundMap>,
}

fn load() -> Installed {
    let mut out = Installed { fences: vec![], grounds: vec![] };
    let Some(root) = std::env::var_os("EO_PARCELS_DIR").map(PathBuf::from) else {
        eprintln!("yards: EO_PARCELS_DIR not set; no fences or yard ground drawn");
        return out;
    };
    let Ok(dir) = std::fs::read_dir(&root) else {
        eprintln!("yards: {} not readable; no fences or yard ground drawn", root.display());
        return out;
    };
    let mut areas: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("yards.json").is_file()).collect();
    areas.sort();
    let scale = eo_data::groundscale::scale();
    for d in areas {
        let name = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let v = std::fs::read(d.join("yards.json")).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string()));
        let Ok(v) = v else {
            eprintln!("yards: {}/yards.json unreadable; skipped", d.display());
            continue;
        };
        let n0 = out.fences.len();
        for line in v["lot_lines"].as_array().into_iter().flatten() {
            for s in line["spans"].as_array().into_iter().flatten() {
                let p = |k: &str| Some([s[k][0].as_f64()?, s[k][1].as_f64()?]);
                let (Some(a), Some(b), Some(h)) = (p("a"), p("b"), s["height_m"].as_f64()) else { continue };
                out.fences.push(Fence { a: eo_data::groundscale::plan(a), b: eo_data::groundscale::plan(b), height_m: h });
            }
        }
        let ground = eo_data::read_maybe_gzip(&d.join("yard-ground.bin.gz")).map_err(|e| e.to_string()).and_then(|b| GroundMap::decode(&b, scale));
        match ground {
            Ok(mut g) => {
                let canopy = d.join("yard-canopy.bin.gz");
                if canopy.is_file() && plants_enabled() {
                    match eo_data::read_maybe_gzip(&canopy).map_err(|e| e.to_string()).and_then(|b| g.decode_canopy(&b, scale)) {
                        Ok(()) => eprintln!("yards: canopy heights and colours from {name}"),
                        Err(e) => eprintln!("yards: {}: {e}; the yards' plants are not drawn", canopy.display()),
                    }
                }
                let a = g.areas();
                eprintln!(
                    "yards: {} fences from {name}; ground {} x {} cells of {:.2} m: lawn {:.0} m2, planting {:.0}, paving {:.0}, deck {:.0} (drawn plan)",
                    out.fences.len() - n0,
                    g.nx,
                    g.nz,
                    g.cell,
                    a[LAWN as usize],
                    a[PLANTING as usize],
                    a[PAVING as usize],
                    a[DECK as usize]
                );
                out.grounds.push(g);
            }
            Err(e) => eprintln!("yards: {} fences from {name}; no ground map ({e})", out.fences.len() - n0),
        }
    }
    out
}

fn installed() -> &'static Installed {
    static I: OnceLock<Installed> = OnceLock::new();
    I.get_or_init(load)
}

/// Every measured fence (none unless `enabled`).
pub fn fences() -> &'static [Fence] {
    if enabled() { &installed().fences } else { &[] }
}

/// Every area's ground classes (none unless `enabled`).
pub fn grounds() -> &'static [GroundMap] {
    if enabled() { &installed().grounds } else { &[] }
}

/// What a yard plant is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlantKind {
    /// A tree of the merged canopy (drawn as a measured tree is, `lidar_trees::model_for`).
    Tree,
    /// A shrub under the trees' floor.
    Shrub,
    /// A small plant in a bed (the planting class at ground level).
    Bed,
    /// A patch of mown turf on the lawn class.
    Lawn,
    /// A clump of rough grass and weeds in a bed's litter (the lawn's blades, taller).
    Weeds,
}

/// One plant the yards' canopy holds where no measured tree stands.
#[derive(Clone, Copy, Debug)]
pub struct YardPlant {
    pub kind: PlantKind,
    /// Position on the drawn plan.
    pub x: f64,
    pub z: f64,
    /// Height (m, real) and crown radius (drawn m).
    pub height_m: f64,
    pub crown_r_m: f64,
    /// The aerial's mean colour over the plant's cells over its class's (the
    /// planting class's median), per channel: the plant's tint against its
    /// species' colour, 1 where the aerial has no colour.
    pub tint: [f32; 3],
    /// A hash of its cell, for its species and turn.
    pub seed: u32,
}

/// Every area's yard plants (none unless `enabled`, or without a canopy map),
/// clear of the measured trees (`lidar_trees::all`, none without `--yard-lidar-trees`).
pub fn plants() -> &'static [YardPlant] {
    static P: OnceLock<Vec<YardPlant>> = OnceLock::new();
    if !enabled() {
        return &[];
    }
    P.get_or_init(|| {
        let trees: Vec<(f64, f64, f64)> = crate::lidar_trees::all().iter().map(|t| (t.x, t.z, t.crown_r_m)).collect();
        let mut out = vec![];
        for g in grounds() {
            let p = fill(g, &trees);
            let n = |k: PlantKind| p.iter().filter(|q| q.kind == k).count();
            if !p.is_empty() {
                eprintln!("yards: canopy plants: {} trees, {} shrubs, {} bed plants, {} lawn patches, {} weed clumps (beside {} measured trees)", n(PlantKind::Tree), n(PlantKind::Shrub), n(PlantKind::Bed), n(PlantKind::Lawn), n(PlantKind::Weeds), trees.len());
            }
            out.extend(p);
        }
        out
    })
}

/// Whether a drawn point is in a yard of a map read with its canopy: the yards
/// draw their trees there (eo-app yards.rs), the measured trees among them too.
pub fn in_yard(x: f64, z: f64) -> bool {
    grounds().iter().any(|g| g.canopy.len() == g.classes.len() && g.at(x, z) != 0)
}

/// The yard plants of one kind in [x0, x1) x [z0, z1) of the drawn plan.
pub fn plants_in_box(kind: PlantKind, x0: f64, z0: f64, x1: f64, z1: f64) -> impl Iterator<Item = &'static YardPlant> {
    plants().iter().filter(move |p| p.kind == kind && p.x >= x0 && p.x < x1 && p.z >= z0 && p.z < z1)
}

fn hash2(i: i64, j: i64, salt: u32) -> u32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841) ^ salt.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f64 {
    (h >> 8) as f64 / (1u32 << 24) as f64
}

/// The plants of one map's canopy, clear of `trees` (x, z, crown radius on the
/// drawn plan). Trees: the local tops of the canopy (smoothed over 3 x 3 cells,
/// at least `TREE_MIN_M`, a top within 0.5 m + a quarter of its height, 1.5 to
/// 5 m), tallest first, each taking the uncovered cells grown downhill from it
/// to `CROWN_BASE_SHARE` of its height within a reach that grows with it; its
/// crown the area-equivalent radius, at its cells' centroid. Shrubs: on a
/// `SHRUB_PITCH_M` grid, every square the uncovered vegetation fills to
/// `SHRUB_FILL`, at its cells' centroid, as tall as its tallest cell. Beds: on a
/// `BED_PITCH_M` grid, `BED_SHARE` of the squares the ground-level planting fills.
pub fn fill(g: &GroundMap, trees: &[(f64, f64, f64)]) -> Vec<YardPlant> {
    if g.canopy.len() != g.classes.len() {
        return vec![];
    }
    let (nx, nz) = (g.nx as i64, g.nz as i64);
    let cells = g.classes.len();
    let idx = |i: i64, j: i64| (i >= 0 && j >= 0 && i < nx && j < nz).then(|| (j * nx + i) as usize);
    let h: Vec<f64> = (0..cells).map(|k| g.height_m(k)).collect();
    let veg: Vec<bool> = (0..cells).map(|k| g.classes[k] != 0 && h[k] >= SHRUB_MIN_M).collect();
    // the planting class's median colour, the tints' reference
    let reference = {
        let mut ch: [Vec<f32>; 3] = Default::default();
        for k in (0..cells).filter(|&k| g.classes[k] == PLANTING) {
            if let Some(c) = g.colour(k) {
                for q in 0..3 {
                    ch[q].push(c[q]);
                }
            }
        }
        ch.map(|mut v| {
            v.sort_by(|a, b| a.total_cmp(b));
            v.get(v.len() / 2).copied().unwrap_or(0.0).max(1e-4)
        })
    };
    let tint_of = |ks: &[usize]| -> [f32; 3] {
        let mut acc = [0f32; 3];
        let mut n = 0.0;
        for &k in ks {
            if let Some(c) = g.colour(k) {
                for q in 0..3 {
                    acc[q] += c[q];
                }
                n += 1.0;
            }
        }
        if n == 0.0 {
            return [1.0; 3];
        }
        [0, 1, 2].map(|q| (acc[q] / n / reference[q]).clamp(0.6, 1.5))
    };
    // covered: within a measured tree's crown
    let mut covered = vec![false; cells];
    let mark_disc = |covered: &mut Vec<bool>, x: f64, z: f64, r: f64| {
        let (i0, i1) = (((x - r - g.x0) / g.cell).floor() as i64, ((x + r - g.x0) / g.cell).ceil() as i64);
        let (j0, j1) = (((z - r - g.z0) / g.cell).floor() as i64, ((z + r - g.z0) / g.cell).ceil() as i64);
        for j in j0.max(0)..=j1.min(nz - 1) {
            for i in i0.max(0)..=i1.min(nx - 1) {
                let k = (j * nx + i) as usize;
                let (cx, cz) = g.centre(k);
                if (cx - x).powi(2) + (cz - z).powi(2) <= r * r {
                    covered[k] = true;
                }
            }
        }
    };
    for &(x, z, r) in trees {
        mark_disc(&mut covered, x, z, r * COVER_SHARE);
    }
    // the smoothed canopy
    let sh: Vec<f64> = (0..cells)
        .map(|k| {
            if !veg[k] {
                return 0.0;
            }
            let (i, j) = ((k as i64) % nx, (k as i64) / nx);
            let (mut s, mut c) = (0.0, 0.0);
            for dj in -1..=1 {
                for di in -1..=1 {
                    if let Some(q) = idx(i + di, j + dj).filter(|&q| veg[q]) {
                        s += h[q];
                        c += 1.0;
                    }
                }
            }
            s / c
        })
        .collect();
    let mut out = vec![];
    // ---- trees: the tops, tallest first
    let mut tops: Vec<usize> = (0..cells)
        .filter(|&k| veg[k] && !covered[k] && sh[k] >= TREE_MIN_M)
        .filter(|&k| {
            let r = (0.5 + 0.25 * sh[k]).clamp(1.5, 5.0);
            let rc = (r / g.cell).ceil() as i64;
            let (i, j) = ((k as i64) % nx, (k as i64) / nx);
            for dj in -rc..=rc {
                for di in -rc..=rc {
                    if (di == 0 && dj == 0) || ((di * di + dj * dj) as f64) * g.cell * g.cell > r * r {
                        continue;
                    }
                    if let Some(q) = idx(i + di, j + dj) {
                        if sh[q] > sh[k] || (sh[q] == sh[k] && q < k) {
                            return false;
                        }
                    }
                }
            }
            true
        })
        .collect();
    tops.sort_by(|a, b| sh[*b].total_cmp(&sh[*a]).then(a.cmp(b)));
    for &k in &tops {
        if covered[k] {
            continue;
        }
        let top = sh[k];
        let reach = (1.5 + 0.35 * top).clamp(2.0, 7.0);
        let (tx, tz) = g.centre(k);
        let mut region = vec![k];
        let mut seen = std::collections::HashSet::from([k]);
        let mut q = std::collections::VecDeque::from([k]);
        while let Some(c) = q.pop_front() {
            let (i, j) = ((c as i64) % nx, (c as i64) / nx);
            for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let Some(m) = idx(i + di, j + dj) else { continue };
                if seen.contains(&m) || !veg[m] || covered[m] || sh[m] < CROWN_BASE_SHARE * top || sh[m] > sh[c] + 0.5 {
                    continue;
                }
                let (mx, mz) = g.centre(m);
                if (mx - tx).hypot(mz - tz) > reach {
                    continue;
                }
                seen.insert(m);
                region.push(m);
                q.push_back(m);
            }
        }
        let area = region.len() as f64 * g.cell * g.cell;
        if area < CROWN_MIN_M2 {
            continue;
        }
        let (mut cx, mut cz) = (0.0, 0.0);
        for &m in &region {
            let (x, z) = g.centre(m);
            cx += x;
            cz += z;
        }
        let (cx, cz) = (cx / region.len() as f64, cz / region.len() as f64);
        let r = (area / std::f64::consts::PI).sqrt();
        for &m in &region {
            covered[m] = true;
        }
        mark_disc(&mut covered, cx, cz, r * COVER_SHARE);
        let (i, j) = ((k as i64) % nx, (k as i64) / nx);
        out.push(YardPlant { kind: PlantKind::Tree, x: cx, z: cz, height_m: h[k].max(top), crown_r_m: r, tint: tint_of(&region), seed: hash2(i, j, 1) });
    }
    // ---- shrubs and beds on their grids
    let grid = |pitch: f64, keep: &dyn Fn(usize) -> bool, out: &mut Vec<(i64, i64, Vec<usize>)>| {
        let (gx, gz) = ((nx as f64 * g.cell / pitch).ceil() as i64, (nz as f64 * g.cell / pitch).ceil() as i64);
        let mut sq: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for k in (0..cells).filter(|&k| keep(k)) {
            let (x, z) = g.centre(k);
            let key = (((x - g.x0) / pitch).floor() as i64, ((z - g.z0) / pitch).floor() as i64);
            if key.0 < gx && key.1 < gz {
                sq.entry(key).or_default().push(k);
            }
        }
        let mut v: Vec<_> = sq.into_iter().map(|((a, b), ks)| (a, b, ks)).collect();
        v.sort_by_key(|e| (e.1, e.0));
        out.extend(v);
    };
    let centroid = |ks: &[usize]| {
        let (mut x, mut z) = (0.0, 0.0);
        for &k in ks {
            let c = g.centre(k);
            x += c.0;
            z += c.1;
        }
        (x / ks.len() as f64, z / ks.len() as f64)
    };
    let mut squares = vec![];
    grid(SHRUB_PITCH_M, &|k| veg[k] && !covered[k], &mut squares);
    for (a, b, ks) in squares {
        let fill = ks.len() as f64 * g.cell * g.cell / (SHRUB_PITCH_M * SHRUB_PITCH_M);
        if fill < SHRUB_FILL {
            continue;
        }
        let (x, z) = centroid(&ks);
        let top = ks.iter().map(|&k| h[k]).fold(0.0, f64::max).clamp(SHRUB_MIN_M, SHRUB_MAX_M);
        let r = (0.5 * SHRUB_PITCH_M * (0.6 + 0.6 * fill.min(1.0))).max(0.45);
        out.push(YardPlant { kind: PlantKind::Shrub, x, z, height_m: top, crown_r_m: r, tint: tint_of(&ks), seed: hash2(a, b, 2) });
    }
    let mut beds = vec![];
    grid(BED_PITCH_M, &|k| g.classes[k] == PLANTING && h[k] < SHRUB_MIN_M && !covered[k], &mut beds);
    for (a, b, ks) in beds {
        let fill = ks.len() as f64 * g.cell * g.cell / (BED_PITCH_M * BED_PITCH_M);
        let s = hash2(a, b, 3);
        if fill < 0.5 || unit(s) >= BED_SHARE {
            continue;
        }
        let (x, z) = centroid(&ks);
        let (jx, jz) = (unit(hash2(a, b, 4)) - 0.5, unit(hash2(a, b, 5)) - 0.5);
        let height = BED_HEIGHT_M.0 + (BED_HEIGHT_M.1 - BED_HEIGHT_M.0) * unit(hash2(a, b, 6));
        out.push(YardPlant { kind: PlantKind::Bed, x: x + jx * 0.4 * BED_PITCH_M, z: z + jz * 0.4 * BED_PITCH_M, height_m: height, crown_r_m: 0.45 * BED_PITCH_M, tint: tint_of(&ks), seed: s });
    }
    let mut lawns = vec![];
    grid(LAWN_PITCH_M, &|k| g.classes[k] == LAWN && h[k] < SHRUB_MIN_M, &mut lawns);
    for (a, b, ks) in lawns {
        let fill = ks.len() as f64 * g.cell * g.cell / (LAWN_PITCH_M * LAWN_PITCH_M);
        if fill < LAWN_FILL {
            continue;
        }
        let (x, z) = centroid(&ks);
        let (jx, jz) = (unit(hash2(a, b, 7)) - 0.5, unit(hash2(a, b, 8)) - 0.5);
        out.push(YardPlant { kind: PlantKind::Lawn, x: x + jx * 0.3 * LAWN_PITCH_M, z: z + jz * 0.3 * LAWN_PITCH_M, height_m: 0.08, crown_r_m: 0.7, tint: tint_of(&ks), seed: hash2(a, b, 9) });
    }
    // the weeds: planting under vegetation (and the open beds, more of them), where the
    // drift field is over its share.
    let drift = |x: f64, z: f64| {
        // value noise on a 3 m lattice, smoothed (0..1).
        let (u, v) = (x / 3.0, z / 3.0);
        let (i, j) = (u.floor() as i64, v.floor() as i64);
        let (fu, fv) = (u - i as f64, v - j as f64);
        let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
        let h = |a: i64, b: i64| unit(hash2(a, b, 11));
        let top = h(i, j) + (h(i + 1, j) - h(i, j)) * su;
        let bot = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * su;
        top + (bot - top) * sv
    };
    let mut weeds = vec![];
    grid(LAWN_PITCH_M, &|k| g.classes[k] == PLANTING, &mut weeds);
    for (a, b, ks) in weeds {
        let (x, z) = centroid(&ks);
        let open = ks.iter().filter(|&&k| h[k] < SHRUB_MIN_M).count() * 2 > ks.len();
        let share = if open { WEEDS_OPEN_SHARE } else { WEEDS_SHARE };
        if drift(x, z) < 1.0 - share || ks.len() as f64 * g.cell * g.cell < 0.5 * LAWN_PITCH_M * LAWN_PITCH_M {
            continue;
        }
        let strength = ((drift(x, z) - (1.0 - share)) / share).clamp(0.0, 1.0);
        let tufts = 3 + (strength * 5.99) as u32;
        for t in 0..tufts {
            let salt = 20 + 4 * t;
            let (jx, jz) = (unit(hash2(a, b, salt)) - 0.5, unit(hash2(a, b, salt + 1)) - 0.5);
            let s = hash2(a, b, salt + 2);
            let r = 0.5 * (WEEDS_TUFT_M.0 + (WEEDS_TUFT_M.1 - WEEDS_TUFT_M.0) * unit(hash2(a, b, salt + 3)));
            let height = WEEDS_HEIGHT_M.0 + (WEEDS_HEIGHT_M.1 - WEEDS_HEIGHT_M.0) * unit(s) * (0.6 + 0.4 * strength);
            out.push(YardPlant { kind: PlantKind::Weeds, x: x + jx * 0.9 * LAWN_PITCH_M, z: z + jz * 0.9 * LAWN_PITCH_M, height_m: height, crown_r_m: r, tint: [1.0; 3], seed: s });
        }
    }
    out
}

/// What a yard object is (eo-render props_lib2 `yard_items`, eo-app yards.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// Two or three collection carts in a row against the house (black, blue, green).
    Carts,
    /// A round table and chairs: weathered teak (`wood`) or a black steel cafe set.
    Patio { wood: bool },
    /// A kettle grill.
    Grill,
    /// A planted container: 0 a terracotta pot, 1 a glazed pot, 2 a half barrel.
    Pot(u8),
    /// A raised vegetable bed (2.4 x 1.2 m).
    RaisedBed,
    /// A small shed, its back to a fence.
    Shed,
}

/// One yard object: its foot on the drawn plan and its heading (three.js
/// rotation.y: local +Z toward (sin yaw, cos yaw)).
#[derive(Clone, Copy, Debug)]
pub struct YardItem {
    pub kind: ItemKind,
    pub x: f64,
    pub z: f64,
    pub yaw: f64,
    pub seed: u32,
}

/// A household's share of the yards: one square of this side (drawn m; a city
/// lot is 25 ft wide and its rear yard about 25-35 ft deep, 7.6-10.7 m, on the
/// 1.25 drawn plan about 10 m) holds at most one of each kind.
pub const ITEM_SQUARE_M: f64 = 10.0;
/// The chance a square with room holds each kind (INFERRED from aerial and
/// street views of the city's rear yards: most keep their carts out back, about
/// half have a table on a patio, fewer a grill, a raised bed or a shed).
pub const ITEM_CHANCE: [(ItemKind, f64); 6] = [
    (ItemKind::Carts, 0.55),
    (ItemKind::Patio { wood: true }, 0.55),
    (ItemKind::Grill, 0.35),
    (ItemKind::Pot(0), 0.5),
    (ItemKind::RaisedBed, 0.3),
    (ItemKind::Shed, 0.14),
];

/// Every area's yard objects (none unless `items_enabled`).
pub fn items() -> &'static [YardItem] {
    static I: OnceLock<Vec<YardItem>> = OnceLock::new();
    if !items_enabled() {
        return &[];
    }
    I.get_or_init(|| {
        let mut out = vec![];
        for g in grounds() {
            let it = place_items(g, fences());
            let n = |f: &dyn Fn(&ItemKind) -> bool| it.iter().filter(|i| f(&i.kind)).count();
            if !it.is_empty() {
                eprintln!(
                    "yards: objects: {} cart rows, {} patio sets, {} grills, {} pots, {} raised beds, {} sheds",
                    n(&|k| *k == ItemKind::Carts),
                    n(&|k| matches!(k, ItemKind::Patio { .. })),
                    n(&|k| *k == ItemKind::Grill),
                    n(&|k| matches!(k, ItemKind::Pot(_))),
                    n(&|k| *k == ItemKind::RaisedBed),
                    n(&|k| *k == ItemKind::Shed)
                );
            }
            // (`EO_YARD_ITEMS_TRACE=1`: each object's kind and place, for placing cameras.)
            if std::env::var("EO_YARD_ITEMS_TRACE").is_ok_and(|v| v == "1") {
                for i in &it {
                    eprintln!("yards: object {:?} at {:.1} {:.1} yaw {:.2}", i.kind, i.x, i.z, i.yaw);
                }
            }
            out.extend(it);
        }
        out
    })
}

/// The yard objects of one map (with its canopy), by `ITEM_SQUARE_M` squares:
/// each kind where the square has room for it, by `ITEM_CHANCE`, clear of the
/// vegetation (a cell under 0.5 m of it), of the other objects and of the
/// yard's edge. Carts stand against a house (a yard cell beside the map's
/// non-yard ground: the buildings), their backs to it; a patio set and a grill
/// on paving or a deck with room round them, the grill by the house where it
/// can; pots in twos to fours on paving along a bed's or a lawn's edge; a
/// raised bed on open bed or lawn ground; a shed with its back to a fence.
/// The bed and the shed take the nearest fence's run (the lots' grid).
pub fn place_items(g: &GroundMap, fences: &[Fence]) -> Vec<YardItem> {
    if g.canopy.len() != g.classes.len() {
        return vec![];
    }
    let (nx, nz) = (g.nx as i64, g.nz as i64);
    let idx = |i: i64, j: i64| (i >= 0 && j >= 0 && i < nx && j < nz).then(|| (j * nx + i) as usize);
    let cell_of = |x: f64, z: f64| (((x - g.x0) / g.cell).floor() as i64, ((z - g.z0) / g.cell).floor() as i64);
    let class_at = |x: f64, z: f64| {
        let (i, j) = cell_of(x, z);
        idx(i, j).map_or(0, |k| g.classes[k])
    };
    let low = |x: f64, z: f64| {
        let (i, j) = cell_of(x, z);
        idx(i, j).is_some_and(|k| g.classes[k] != 0 && g.height_m(k) < SHRUB_MIN_M)
    };
    // every sample within r (on a half-cell lattice) passes `ok`.
    let clear = |x: f64, z: f64, r: f64, ok: &dyn Fn(f64, f64) -> bool| {
        let step = g.cell * 0.5;
        let n = (r / step).ceil() as i64;
        for a in -n..=n {
            for b in -n..=n {
                let (dx, dz) = (a as f64 * step, b as f64 * step);
                if dx * dx + dz * dz <= r * r && !ok(x + dx, z + dz) {
                    return false;
                }
            }
        }
        true
    };
    let hard = |x: f64, z: f64| low(x, z) && matches!(class_at(x, z), PAVING | DECK);
    // the direction away from the non-yard ground within 1 m (a house's wall), if any
    // (the map's own edge is not a wall).
    let built = |x: f64, z: f64| {
        let (i, j) = cell_of(x, z);
        idx(i, j).is_some_and(|k| g.classes[k] == 0)
    };
    let wall = |x: f64, z: f64| -> Option<(f64, f64)> {
        let (mut sx, mut sz, mut n) = (0.0, 0.0, 0);
        let r = 1.0;
        let step = g.cell;
        let m = (r / step).ceil() as i64;
        for a in -m..=m {
            for b in -m..=m {
                let (dx, dz) = (a as f64 * step, b as f64 * step);
                if (a, b) != (0, 0) && dx * dx + dz * dz <= r * r && built(x + dx, z + dz) {
                    sx -= dx;
                    sz -= dz;
                    n += 1;
                }
            }
        }
        let l = sx.hypot(sz);
        (n >= 2 && l > 1e-6).then(|| (sx / l, sz / l))
    };
    // the nearest fence within `r`: its run's direction and the side's normal toward (x, z), its distance.
    let fence_near = |x: f64, z: f64, r: f64| -> Option<((f64, f64), (f64, f64), f64)> {
        let mut best: Option<((f64, f64), (f64, f64), f64)> = None;
        for f in fences {
            let (dx, dz) = (f.b[0] - f.a[0], f.b[1] - f.a[1]);
            let len = dx.hypot(dz);
            if len < 1.0 {
                continue;
            }
            let (tx, tz) = (dx / len, dz / len);
            let s = ((x - f.a[0]) * tx + (z - f.a[1]) * tz).clamp(0.0, len);
            let (px, pz) = (f.a[0] + tx * s, f.a[1] + tz * s);
            let d = (x - px).hypot(z - pz);
            if d <= r && best.is_none_or(|b| d < b.2) {
                let (mut nx_, mut nz_) = (-tz, tx);
                if (x - px) * nx_ + (z - pz) * nz_ < 0.0 {
                    nx_ = -nx_;
                    nz_ = -nz_;
                }
                best = Some(((tx, tz), (nx_, nz_), d));
            }
        }
        best
    };
    // clear of every fence by r (a fence stands on the lot line, the neighbour's side beyond it).
    let off_fence = |x: f64, z: f64, r: f64| fence_near(x, z, r).is_none();
    let mut out: Vec<YardItem> = vec![];
    let mut taken: Vec<(f64, f64, f64)> = vec![];
    let free_of = |taken: &[(f64, f64, f64)], x: f64, z: f64, r: f64| taken.iter().all(|&(a, b, q)| (a - x).hypot(b - z) >= q + r);
    let (sx, sz) = ((nx as f64 * g.cell / ITEM_SQUARE_M).ceil() as i64, (nz as f64 * g.cell / ITEM_SQUARE_M).ceil() as i64);
    for b in 0..sz {
        for a in 0..sx {
            let (x0, z0) = (g.x0 + a as f64 * ITEM_SQUARE_M, g.z0 + b as f64 * ITEM_SQUARE_M);
            // the square's candidate points: its cells' centres in a seeded order.
            let mut pts: Vec<(f64, f64, u32)> = vec![];
            let per = (ITEM_SQUARE_M / g.cell).round() as i64;
            let (i0, j0) = cell_of(x0 + 1e-6, z0 + 1e-6);
            for dj in 0..per {
                for di in 0..per {
                    if let Some(k) = idx(i0 + di, j0 + dj) {
                        if g.classes[k] != 0 {
                            let (x, z) = g.centre(k);
                            pts.push((x, z, hash2(i0 + di, j0 + dj, 41)));
                        }
                    }
                }
            }
            if pts.len() < 20 {
                continue;
            }
            pts.sort_by_key(|p| p.2);
            let roll = |salt: u32| unit(hash2(a, b, salt));
            let seed = hash2(a, b, 47);
            let put = |out: &mut Vec<YardItem>, taken: &mut Vec<(f64, f64, f64)>, kind: ItemKind, x: f64, z: f64, yaw: f64, r: f64, salt: u32| {
                out.push(YardItem { kind, x, z, yaw, seed: seed ^ salt.wrapping_mul(0x9e37_79b9) });
                taken.push((x, z, r));
            };
            // carts: back to the house, 0.45 m out from it, room for a row of three (1.9 m).
            if roll(50) < ITEM_CHANCE[0].1 {
                if let Some((x, z, (wx, wz))) = pts.iter().find_map(|&(x, z, _)| {
                    let w = wall(x, z)?;
                    let (cx, cz) = (x + w.0 * 0.35, z + w.1 * 0.35);
                    let row = (-1..=1).all(|k| {
                        let (px, pz) = (cx - w.1 * 0.65 * k as f64, cz + w.0 * 0.65 * k as f64);
                        clear(px, pz, 0.32, &|u, v| low(u, v))
                    });
                    (row && free_of(&taken, cx, cz, 1.0)).then_some((cx, cz, w))
                }) {
                    put(&mut out, &mut taken, ItemKind::Carts, x, z, (-wx).atan2(-wz), 1.0, 1);
                }
            }
            // a patio set with 1.1 m round it on paving or a deck.
            if roll(51) < ITEM_CHANCE[1].1 {
                if let Some(&(x, z, h)) = pts.iter().find(|&&(x, z, _)| hard(x, z) && off_fence(x, z, 1.2) && clear(x, z, 1.1, &|u, v| low(u, v) && class_at(u, v) != PLANTING) && free_of(&taken, x, z, 1.1)) {
                    put(&mut out, &mut taken, ItemKind::Patio { wood: unit(h) < 0.65 }, x, z, unit(h ^ 0x55) * std::f64::consts::TAU, 1.1, 2);
                }
            }
            // a grill on paving, by the house if it can.
            if roll(52) < ITEM_CHANCE[2].1 {
                let fits = |x: f64, z: f64| hard(x, z) && off_fence(x, z, 0.6) && clear(x, z, 0.45, &|u, v| low(u, v)) && free_of(&taken, x, z, 0.6);
                let by_house = pts.iter().find(|&&(x, z, _)| fits(x, z) && wall(x, z).is_some()).copied();
                if let Some((x, z, h)) = by_house.or_else(|| pts.iter().find(|&&(x, z, _)| fits(x, z)).copied()) {
                    let yaw = wall(x, z).map_or(unit(h) * std::f64::consts::TAU, |(wx, wz)| (-wx).atan2(-wz));
                    put(&mut out, &mut taken, ItemKind::Grill, x, z, yaw, 0.6, 3);
                }
            }
            // pots: two to four on paving along a bed's or a lawn's edge, 0.6-1.4 m apart.
            if roll(53) < ITEM_CHANCE[3].1 {
                let edge = |x: f64, z: f64| hard(x, z) && off_fence(x, z, 0.45) && !clear(x, z, 0.7, &|u, v| matches!(class_at(u, v), PAVING | DECK)) && clear(x, z, 0.28, &|u, v| low(u, v));
                if let Some(&(ax, az, _)) = pts.iter().find(|&&(x, z, _)| edge(x, z) && free_of(&taken, x, z, 0.35)) {
                    let want = 2 + (roll(54) * 2.99) as usize;
                    let mut got = 0;
                    for &(x, z, h) in pts.iter().filter(|&&(x, z, _)| (x - ax).hypot(z - az) <= 1.6) {
                        if got >= want {
                            break;
                        }
                        if edge(x, z) && free_of(&taken, x, z, 0.3) {
                            let k = match unit(h ^ 0x77) {
                                u if u < 0.55 => 0,
                                u if u < 0.8 => 1,
                                _ => 2,
                            };
                            put(&mut out, &mut taken, ItemKind::Pot(k), x, z, unit(h) * std::f64::consts::TAU, 0.32, 10 + got as u32);
                            got += 1;
                        }
                    }
                }
            }
            // a raised bed on open ground (a bed's or a lawn's), along the lots' grid.
            if roll(55) < ITEM_CHANCE[4].1 {
                if let Some(&(x, z, _)) = pts.iter().find(|&&(x, z, _)| matches!(class_at(x, z), PLANTING | LAWN) && off_fence(x, z, 1.4) && clear(x, z, 1.2, &|u, v| low(u, v)) && free_of(&taken, x, z, 1.4)) {
                    let yaw = fence_near(x, z, 12.0).map_or(0.0, |((tx, tz), _, _)| tx.atan2(tz));
                    put(&mut out, &mut taken, ItemKind::RaisedBed, x, z, yaw, 1.4, 4);
                }
            }
            // a shed, its back 0.1 m off a fence.
            if roll(56) < ITEM_CHANCE[5].1 {
                if let Some((x, z, yaw)) = pts.iter().find_map(|&(x, z, _)| {
                    let (_, (fx, fz), d) = fence_near(x, z, 2.0)?;
                    // the shed's centre 0.7 m off the fence line, on this side (room for its 1.8 x 1.2 m).
                    let (cx, cz) = (x + fx * (0.7 - d), z + fz * (0.7 - d));
                    (clear(cx, cz, 0.85, &|u, v| low(u, v)) && free_of(&taken, cx, cz, 1.2)).then_some((cx, cz, (-fx).atan2(-fz)))
                }) {
                    put(&mut out, &mut taken, ItemKind::Shed, x, z, yaw, 1.2, 5);
                }
            }
        }
    }
    out
}

/// One drawn piece of a fence: its foot's centre on the drawn ground (less
/// `SINK_M`), its heading (local +Z along the fence, three.js rotation.y), its
/// length, its height from the foot and its grade along (+Z, rise over run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FencePiece {
    pub pos: [f64; 3],
    pub yaw: f64,
    pub length: f64,
    pub height: f64,
    pub slope: f64,
}

/// A fence cut into pieces of at most `PIECE_MAX_M`, each following the drawn
/// ground between its ends, left out where `blocked` (a drawn building or the
/// drawn street) holds a sample of it.
pub fn pieces(f: &Fence, ground: &dyn Fn(f64, f64) -> f64, blocked: &dyn Fn(f64, f64) -> bool) -> Vec<FencePiece> {
    let (dx, dz) = (f.b[0] - f.a[0], f.b[1] - f.a[1]);
    let len = dx.hypot(dz);
    if len < SAMPLE_M {
        return vec![];
    }
    let (tx, tz) = (dx / len, dz / len);
    let yaw = tx.atan2(tz);
    let at = |s: f64| (f.a[0] + tx * s, f.a[1] + tz * s);
    // free runs of samples
    let steps = (len / SAMPLE_M).ceil() as usize;
    let free: Vec<bool> = (0..=steps).map(|k| {
        let (x, z) = at((k as f64 * SAMPLE_M).min(len));
        !blocked(x, z)
    }).collect();
    let mut out = vec![];
    let mut k = 0;
    while k <= steps {
        if !free[k] {
            k += 1;
            continue;
        }
        let k0 = k;
        while k + 1 <= steps && free[k + 1] {
            k += 1;
        }
        let (s0, s1) = ((k0 as f64 * SAMPLE_M).min(len), (k as f64 * SAMPLE_M).min(len));
        let run = s1 - s0;
        if run >= SAMPLE_M {
            let n = (run / PIECE_MAX_M).ceil().max(1.0) as usize;
            let piece = run / n as f64;
            for p in 0..n {
                let (a, b) = (s0 + p as f64 * piece, s0 + (p + 1) as f64 * piece);
                let ((xa, za), (xb, zb), (xc, zc)) = (at(a), at(b), at((a + b) / 2.0));
                let (ga, gb) = (ground(xa, za), ground(xb, zb));
                out.push(FencePiece { pos: [xc, (ga + gb) / 2.0 - SINK_M, zc], yaw, length: piece, height: f.height_m + SINK_M, slope: (gb - ga) / piece });
            }
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_follow_the_ground_and_skip_what_is_blocked() {
        let f = Fence { a: [0.0, 0.0], b: [0.0, 10.0], height_m: 1.8 };
        let ground = |_x: f64, z: f64| 0.1 * z;
        let ps = pieces(&f, &ground, &|_, z| (4.0..6.0).contains(&z));
        // two free runs (0-3.75 m and 6-10 m), each in pieces of at most PIECE_MAX_M.
        let total: f64 = ps.iter().map(|p| p.length).sum();
        assert!((total - (3.75 + 4.0)).abs() < 1e-9, "{total}");
        assert!(ps.iter().all(|p| p.length <= PIECE_MAX_M + 1e-9));
        for p in &ps {
            assert!((p.slope - 0.1).abs() < 1e-9);
            assert!((p.pos[1] - (0.1 * p.pos[2] - SINK_M)).abs() < 1e-9, "the foot's centre sits SINK_M under the ground");
            assert!((p.height - (1.8 + SINK_M)).abs() < 1e-9, "the top stays at the measured height");
            assert!((p.yaw - 0.0).abs() < 1e-9, "local +Z along +z");
            assert!(!(4.0..6.0).contains(&p.pos[2]));
        }
    }

    #[test]
    fn ground_map_round_trip() {
        let mut b = GROUND_MAGIC.to_vec();
        for v in [3u32, 2] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [10.0f32, 20.0, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0, LAWN, PLANTING, PAVING, DECK, 0]);
        let g = GroundMap::decode(&b, 1.25).unwrap();
        assert_eq!((g.nx, g.nz), (3, 2));
        assert!((g.cell - 0.625).abs() < 1e-9 && (g.x0 - 12.5).abs() < 1e-9 && (g.z0 - 25.0).abs() < 1e-9);
        assert_eq!(g.at(12.5 + 0.625 * 1.5, 25.1), LAWN);
        assert_eq!(g.at(12.5 + 0.625 * 0.5, 25.0 + 0.625 * 1.5), PAVING);
        assert_eq!(g.at(0.0, 0.0), 0);
        b.push(0);
        assert!(GroundMap::decode(&b, 1.0).is_err(), "a cell count that does not match");
    }

    fn canopy_bytes(nx: u32, nz: u32, x0: f32, z0: f32, cell: f32, cells: &[[u8; 5]]) -> Vec<u8> {
        let mut b = CANOPY_MAGIC.to_vec();
        for v in [nx, nz] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [x0, z0, cell] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for c in cells {
            b.extend_from_slice(c);
        }
        b
    }

    fn with_class(cells: &[[u8; 4]], classes: &[u8]) -> Vec<[u8; 5]> {
        cells.iter().zip(classes).map(|(c, &k)| [c[0], c[1], c[2], c[3], k]).collect()
    }

    #[test]
    fn canopy_reads_onto_its_ground_map_only() {
        let mut g = GroundMap { x0: 12.5, z0: 25.0, cell: 0.625, nx: 2, nz: 1, classes: vec![PLANTING, LAWN], canopy: vec![] };
        let b = canopy_bytes(2, 1, 10.0, 20.0, 0.5, &with_class(&[[35, 60, 80, 40], [0, 100, 110, 90]], &[PLANTING, PAVING]));
        g.decode_canopy(&b, 1.25).unwrap();
        assert_eq!(g.classes, vec![PLANTING, PAVING], "the canopy's classes (paving squared) replace the ground file's");
        assert!((g.height_m(0) - 3.5).abs() < 1e-9 && g.height_m(1) == 0.0);
        let c = g.colour(0).unwrap();
        assert!(c[1] > c[0] && c[1] > c[2], "the aerial's green over red and blue");
        let mut other = GroundMap { canopy: vec![], ..g.clone() };
        assert!(other.decode_canopy(&canopy_bytes(2, 1, 11.0, 20.0, 0.5, &[[0; 5]; 2]), 1.25).is_err(), "another grid");
        assert!(other.decode_canopy(&canopy_bytes(2, 1, 10.0, 20.0, 0.5, &[[0; 5]; 1]), 1.25).is_err(), "too few cells");
    }

    /// A 30 m square: a tall crown (7 m) to the west, a measured tree's crown
    /// to the east, a low hedge (1.5 m) to the south, a bed at ground level.
    #[test]
    fn the_canopy_fills_with_trees_shrubs_and_bed_plants() {
        let (n, cell) = (60u32, 0.5);
        let mut classes = vec![0u8; (n * n) as usize];
        let mut canopy = vec![[0u8; 4]; (n * n) as usize];
        let at = |x: f64, z: f64| ((z / cell) as usize) * n as usize + (x / cell) as usize;
        let mut paint = |x0: f64, z0: f64, x1: f64, z1: f64, class: u8, h: f64, f: &dyn Fn(f64, f64) -> f64| {
            let mut z = z0;
            while z < z1 {
                let mut x = x0;
                while x < x1 {
                    let k = at(x, z);
                    classes[k] = class;
                    canopy[k] = [((h * f(x, z)) * 10.0).round() as u8, 70, 90, 50];
                    x += cell;
                }
                z += cell;
            }
        };
        let dome = |cx: f64, cz: f64, r: f64| move |x: f64, z: f64| (1.0 - ((x - cx).hypot(z - cz) / r).powi(2)).max(0.3);
        paint(2.0, 2.0, 10.0, 10.0, PLANTING, 7.0, &dome(6.0, 6.0, 4.0));
        paint(18.0, 2.0, 26.0, 10.0, PLANTING, 7.0, &dome(22.0, 6.0, 4.0));
        paint(2.0, 20.0, 20.0, 22.0, PLANTING, 1.5, &|_, _| 1.0);
        paint(22.0, 20.0, 28.0, 28.0, PLANTING, 0.0, &|_, _| 1.0);
        let g = GroundMap { x0: 0.0, z0: 0.0, cell, nx: n, nz: n, classes, canopy };
        let measured = [(22.0, 6.0, 4.0)];
        let p = fill(&g, &measured);
        let trees: Vec<_> = p.iter().filter(|q| q.kind == PlantKind::Tree).collect();
        assert_eq!(trees.len(), 1, "one tree for the free crown, none under the measured one: {trees:?}");
        let t = trees[0];
        assert!((t.x - 6.0).abs() < 1.0 && (t.z - 6.0).abs() < 1.0 && t.height_m >= 6.5, "{t:?}");
        assert!(t.crown_r_m > 2.0 && t.crown_r_m < 4.6, "{t:?}");
        let shrubs: Vec<_> = p.iter().filter(|q| q.kind == PlantKind::Shrub).collect();
        let hedge: Vec<_> = shrubs.iter().filter(|s| s.z > 19.0 && s.z < 23.0).collect();
        assert!(hedge.len() >= 6 && hedge.iter().all(|s| (s.height_m - 1.5).abs() < 1e-9), "the hedge's shrubs at its height: {hedge:?}");
        // the rest stand round the crowns' low fringes (under CROWN_BASE_SHARE of the top,
        // or past the measured crown's covered share), never in a crown's core.
        for s in shrubs.iter().filter(|s| s.z < 19.0) {
            assert!(s.height_m <= SHRUB_MAX_M + 1e-9);
            assert!((s.x - 6.0).hypot(s.z - 6.0) > 1.5 && (s.x - 22.0).hypot(s.z - 6.0) > 0.85 * 4.0 - 0.5, "{s:?}");
        }
        let beds: Vec<_> = p.iter().filter(|q| q.kind == PlantKind::Bed).collect();
        assert!(!beds.is_empty() && beds.iter().all(|b| b.x > 21.0 && b.z > 19.0 && b.height_m >= BED_HEIGHT_M.0 && b.height_m <= BED_HEIGHT_M.1), "{beds:?}");
        assert!(p.iter().all(|q| q.tint.iter().all(|v| (v - 1.0).abs() < 1e-4)), "one colour everywhere: the tints are 1");
        assert!(fill(&GroundMap { canopy: vec![], ..g.clone() }, &[]).is_empty(), "no canopy map: no plants");
    }

    #[test]
    fn objects_stand_in_the_yards_clear_of_plants_and_each_other() {
        // 60 x 60 cells of 0.625 m: a house (no yard) on the west 8 columns, paving
        // to x 20 m, a lawn beyond, and a band of shrubs 1.5 m tall along the north.
        let (n, cell) = (60u32, 0.625);
        let mut classes = vec![0u8; (n * n) as usize];
        let mut canopy = vec![[0u8; 4]; (n * n) as usize];
        for j in 0..n {
            for i in 0..n {
                let k = (j * n + i) as usize;
                let x = (i as f64 + 0.5) * cell;
                classes[k] = if i < 8 { 0 } else if x < 20.0 { PAVING } else { LAWN };
                if j < 4 && i >= 8 {
                    classes[k] = PLANTING;
                    canopy[k][0] = 15;
                }
            }
        }
        let g = GroundMap { x0: 0.0, z0: 0.0, cell, nx: n, nz: n, classes, canopy };
        let items = place_items(&g, &[Fence { a: [37.0, 0.0], b: [37.0, 37.5], height_m: 1.8 }]);
        assert!(!items.is_empty(), "a yard with room holds objects");
        for it in &items {
            assert!(g.at(it.x, it.z) != 0, "{:?} at ({:.1}, {:.1}) off the yard", it.kind, it.x, it.z);
            let (i, j) = ((it.x / cell) as u32, (it.z / cell) as u32);
            assert!(g.height_m((j * n + i) as usize) < SHRUB_MIN_M, "{:?} on a shrub", it.kind);
        }
        // carts with their backs to the house: facing east (+x), away from it.
        for c in items.iter().filter(|i| i.kind == ItemKind::Carts) {
            assert!(c.x < 5.0 + 2.0, "carts by the house, not at x {:.1}", c.x);
            assert!((-c.yaw.sin() - 1.0).abs() < 0.3, "carts' fronts face away from the house (yaw {:.2})", c.yaw);
        }
        for (a, b) in items.iter().zip(items.iter().skip(1)) {
            assert!((a.x - b.x).hypot(a.z - b.z) > 0.3, "{:?} and {:?} on one spot", a.kind, b.kind);
        }
        assert!(items.iter().any(|i| matches!(i.kind, ItemKind::Patio { .. } | ItemKind::Grill)), "paving holds a table or a grill");
        assert!(place_items(&GroundMap { canopy: vec![], ..g.clone() }, &[]).is_empty(), "no canopy map: no objects");
    }

}
