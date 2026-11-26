//! The terrain surface pass (round 3): the ground's relief at 2 m from the
//! USGS 3DEP 1 m DEM and its materials from the USDA NAIP 0.6 m imagery,
//! over the open ground round the photo views (child of `scene`; the survey
//! is read through `eo_farshore::survey`, `EO_SURVEY_ROOT`; with it absent,
//! or a file not matching its manifest digest, the terrain draws as before).
//!
//! **Relief** (`--no-terrain-detail`). Round each photo eye (`REGIONS`, the
//! eyes of the four photo views and the bluff, which are also the starts of
//! the bench's headland, bluff, baker and crissy_beach walks) the 8 m terrain
//! grid is replaced by patches of 64 m (8 x 8 grid cells) at a 2 m step: the
//! drawn 8 m surface (the same two triangles a cell is drawn with) plus the
//! high-pass of the 1 m DEM, `(DEM - its 8 m low-pass) x height scale`, faded
//! to nothing at the patches' outer edge (so an outer patch edge lies exactly
//! on the 8 m triangles' edges: no crack) and at paving, kerbs, buildings,
//! the street world's surfaces, beach sand, the mapped lawns and parks (every
//! ground-cover tone that is not open ground, `OPEN_TONES`: mown ground keeps
//! the 8 m surface, and the plants' classification with it), pale bare
//! ground in the NAIP (concrete slabs, gravel, paths: `pale_ground`), round
//! the photo eyes (`EYE_CLEAR_M`: the frozen eyes stand 1.7 m over the 8 m
//! ground) and wherever the street carpet or the shore press moved the
//! ground (so nothing floats or sinks). The 8 m
//! low-pass is the DEM averaged over a grid cell round each 8 m vertex, drawn
//! with the cell's two triangles. The terrain tiles skip the cells a patch
//! covers, at both levels (patches are whole coarse cells). A patch with no
//! relief and no survey material is dropped (the tiles draw it), and its
//! neighbours are rebuilt so their edges toward it carry no relief. The
//! patches' shading normals are the tiles' own (interpolated over the 8 m
//! triangles) bent by the relief's gradient, so ground without relief shades
//! exactly as the tiles do. Heights go to `eo_data::detail_ground` where the
//! relief moved the ground (None elsewhere: the readers keep their own 8 m
//! ground) so the props sit on them.
//!
//! **The city's hills** (`--no-city-relief`, Docs/HILLS-PLAN.md H3): with the hill builder's
//! cooked relief (`cooked`, eo-hills `relief`) a patch is also drawn over every block with
//! at least `COOKED_RELIEF_MIN` of relief, at most `COOKED_BUILT_MAX` under footprints and
//! at least `COOKED_GREEN_MIN` open ground or mapped greens, its relief read from the block
//! (the same high-pass, cooked from the six 1 m tiles) instead of the survey; such a patch
//! takes relief on the parks' mapped grass too. Without the survey, the patches round the
//! photo eyes are not drawn. "Pressed" ground is ground the carpet moved away from the
//! patch's median offset (the carpet lifts all open ground by one constant).
//!
//! **Materials** (`--no-terrain-materials`): see `Materials`.

use crate::scene::Vertex;
use eo_data::geo::GeoFrame;
use eo_farshore::survey::{Root, dem1m::Dem1m, naip::Naip};
use std::collections::{HashMap, HashSet};

/// Terrain grid cells per patch side (64 m at the 8 m grid).
pub const PATCH_CELLS: i64 = 8;
/// Detail steps per grid cell (2 m at the 8 m grid).
pub const STEPS: i64 = 4;
/// Vertices per patch side (one patch = the terrain tile's 33 x 33 topology).
pub const SIDE: usize = (PATCH_CELLS * STEPS + 1) as usize;
/// Half the side of the square of patches round each eye, game metres
/// (`about 400 m round each photo eye`, the track's brief).
pub const REGION_HALF_M: f64 = 400.0;
/// The detail fades out over this distance to the patches' outer edge, m.
pub const EDGE_FADE_M: f64 = 24.0;
/// ... and over this distance from paving, kerbs, buildings, the street
/// world's surfaces, sand and pressed ground, m (INFERRED: three detail steps).
pub const MASK_FADE_M: f64 = 6.0;
/// The ground the street carpet or the shore press moved by more than this
/// (game m) is held by them: no detail there.
pub const PRESSED_M: f32 = 0.05;
/// The largest relief added, game m: a DEM step beyond it is a survey
/// artefact (a wall or a building edge left in the bare-earth model).
pub const MAX_RELIEF_M: f32 = 4.0;
/// No relief within this distance of a photo eye (game m; it fades back in
/// over `MASK_FADE_M`). The views' eyes are frozen (views-r3.json) at 1.7 m
/// over the 8 m ground they were fitted on; the 1 m DEM puts the battery
/// slab's real edge about 9 m ahead of the headland eye, which the relief
/// raises 1.2-1.4 m and which then hides the view. INFERRED: the slab edge's
/// distance plus two detail steps; held until the eyes are re-fitted on the
/// 1 m ground (an owner's decision).
pub const EYE_CLEAR_M: f64 = 15.0;
/// A relief smaller than this (game m) is none: the provider answers None there.
pub const RELIEF_EPS_M: f32 = 0.01;

/// The eyes the relief is drawn round, game metres (x, z): the photo views'
/// eyes (views-r3.json), which are the bench walks' starts too.
pub const REGIONS: [(&str, [f64; 2]); 5] = [
    ("headland", [-6836.85, -6849.47]),
    ("bluff", [-6093.05, -4174.17]),
    ("baker", [-6752.5, -3033.87]),
    ("crissy_beach", [-5040.51, -3745.69]),
    ("fort_point", [-5785.1, -4240.51]),
];

/// The terrain grid's lattice: vertex (cx, cz) at (`ox + (cx + 0.5) cell`, ...).
#[derive(Clone, Copy, Debug)]
pub struct Lattice {
    pub ox: f64,
    pub oz: f64,
    pub cell: f64,
    pub width: i64,
    pub height: i64,
}

impl Lattice {
    pub fn vx(&self, cx: i64) -> f64 {
        self.ox + (cx as f64 + 0.5) * self.cell
    }
    pub fn vz(&self, cz: i64) -> f64 {
        self.oz + (cz as f64 + 0.5) * self.cell
    }
    /// The patch holding grid quad (cx, cz).
    pub fn patch_of_quad(cx: i64, cz: i64) -> (i64, i64) {
        (cx.div_euclid(PATCH_CELLS), cz.div_euclid(PATCH_CELLS))
    }
}

/// A height on one grid cell's two triangles (the terrain tiles' split: the
/// diagonal from (1, 0) to (0, 1)), at (u, v) in [0, 1]^2.
pub fn cell_tri(h00: f32, h10: f32, h01: f32, h11: f32, u: f32, v: f32) -> f32 {
    if u + v <= 1.0 { h00 + u * (h10 - h00) + v * (h01 - h00) } else { h11 + (1.0 - u) * (h01 - h11) + (1.0 - v) * (h10 - h11) }
}

/// Whether patch `key` lies in the square round a photo eye that `patch_keys` covers.
fn near_region(l: &Lattice, key: (i64, i64), eye: &[f64; 2]) -> bool {
    let pm = PATCH_CELLS as f64 * l.cell;
    let (cx, cz) = (l.ox + (key.0 as f64 + 0.5) * pm, l.oz + (key.1 as f64 + 0.5) * pm);
    (cx - eye[0]).abs() <= REGION_HALF_M + pm && (cz - eye[1]).abs() <= REGION_HALF_M + pm
}

/// The patches chosen round `REGIONS` that lie inside the grid.
pub fn patch_keys(l: &Lattice) -> Vec<(i64, i64)> {
    let mut set = HashSet::new();
    let pm = PATCH_CELLS as f64 * l.cell;
    for (_, c) in REGIONS {
        let (x0, x1) = (c[0] - REGION_HALF_M, c[0] + REGION_HALF_M);
        let (z0, z1) = (c[1] - REGION_HALF_M, c[1] + REGION_HALF_M);
        // patch p covers vertices cx in [8p, 8p + 8]
        let pxa = ((x0 - l.ox) / pm).floor() as i64;
        let pxb = ((x1 - l.ox) / pm).ceil() as i64;
        let pza = ((z0 - l.oz) / pm).floor() as i64;
        let pzb = ((z1 - l.oz) / pm).ceil() as i64;
        for pz in pza..pzb {
            for px in pxa..pxb {
                if px >= 0 && pz >= 0 && (px + 1) * PATCH_CELLS < l.width && (pz + 1) * PATCH_CELLS < l.height {
                    set.insert((px, pz));
                }
            }
        }
    }
    let mut v: Vec<_> = set.into_iter().collect();
    v.sort();
    v
}

/// One patch: its vertices (SIDE x SIDE, row-major from its north-west corner).
pub struct Patch {
    pub key: (i64, i64),
    pub verts: Vec<Vertex>,
    /// each vertex's relief over the 8 m surface (m)
    pub relief: Vec<f32>,
    /// vertices with relief over 1 cm, and the largest |relief| (m)
    pub relieved: usize,
    pub max_relief_m: f32,
    /// vertices carrying a survey material
    pub material: usize,
}

/// The built relief: the patches, and their heights for `detail_ground`.
pub struct Detail {
    pub patches: Vec<Patch>,
    pub keys: HashSet<(i64, i64)>,
    pub lattice: Lattice,
    pub stats: DetailStats,
    index: HashMap<(i64, i64), usize>,
}

#[derive(Clone, Debug, Default)]
pub struct DetailStats {
    pub patches: usize,
    /// patches dropped (no relief, no material: the tiles draw them)
    pub dropped: usize,
    /// patches rebuilt because a neighbour was dropped
    pub rebuilt: usize,
    pub vertices: usize,
    pub relieved: usize,
    pub max_relief_m: f32,
    pub ms: f64,
}

impl Detail {
    /// Whether grid quad (cx, cz) is drawn by a patch.
    pub fn covers_quad(&self, cx: i64, cz: i64) -> bool {
        self.keys.contains(&Lattice::patch_of_quad(cx, cz))
    }

    /// The drawn height at (x, z) where a patch draws it, whatever its relief.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        self.sample(x, z).map(|(h, _)| h)
    }

    /// The drawn height at (x, z) where the relief moved the ground (a corner
    /// of the 2 m triangle under it by `RELIEF_EPS_M` or more); None where a
    /// patch draws the 8 m surface unchanged (masked, faded, the sea) or no
    /// patch is: there the readers' own ground (the 8 m triangles, a rock's
    /// footprint minimum) is the drawn one.
    pub fn relief_height(&self, x: f64, z: f64) -> Option<f64> {
        self.sample(x, z).filter(|&(_, r)| r >= RELIEF_EPS_M).map(|(h, _)| h)
    }

    /// (height, the largest |relief| at the triangle's corners)
    fn sample(&self, x: f64, z: f64) -> Option<(f64, f32)> {
        let l = &self.lattice;
        let step = l.cell / STEPS as f64;
        let fx = (x - l.vx(0)) / step;
        let fz = (z - l.vz(0)) / step;
        if !(fx >= 0.0 && fz >= 0.0) {
            return None;
        }
        let (ix, iz) = (fx.floor() as i64, fz.floor() as i64);
        let n = PATCH_CELLS * STEPS;
        let key = (ix.div_euclid(n), iz.div_euclid(n));
        let p = &self.patches[*self.index.get(&key)?];
        let (i, j) = ((ix - key.0 * n) as usize, (iz - key.1 * n) as usize);
        let (u, v) = ((fx - ix as f64) as f32, (fz - iz as f64) as f32);
        let k = |a: usize, b: usize| b.min(SIDE - 1) * SIDE + a.min(SIDE - 1);
        let h = |a: usize, b: usize| p.verts[k(a, b)].pos[1];
        let r = [k(i, j), k(i + 1, j), k(i, j + 1), k(i + 1, j + 1)].iter().map(|&q| p.relief[q].abs()).fold(0.0f32, f32::max);
        Some((cell_tri(h(i, j), h(i + 1, j), h(i, j + 1), h(i + 1, j + 1), u, v) as f64, r))
    }
}

/// What the patches are made from (scene.rs fills it from its inputs).
pub struct Inputs<'a> {
    pub lattice: Lattice,
    /// The terrain vertex height at grid vertex (cx, cz), exactly as the tiles draw it.
    pub vertex_height: &'a (dyn Fn(i64, i64) -> f32 + Sync),
    /// The raw ground (the elevation grid) at (x, z): where the drawn ground
    /// differs, the carpet or the press holds it.
    pub raw_ground: &'a (dyn Fn(f64, f64) -> f32 + Sync),
    /// A vertex's colour at (x, z), as the tiles colour theirs (the alpha
    /// byte: < 254 beach sand).
    pub colour: &'a (dyn Fn(f32, f32) -> u32 + Sync),
    /// Plan triangles (x, z) of the street world's static surfaces.
    pub street_tris: &'a [[[f32; 2]; 3]],
    /// Building footprint rings.
    pub buildings: Vec<&'a [[f32; 2]]>,
    pub frame: GeoFrame,
    pub height_scale: f64,
    pub sea_y: f32,
}

/// The survey's 1 m DEM, opened once (None: no survey root, or refused).
pub fn dem() -> Option<&'static Dem1m> {
    static D: std::sync::OnceLock<Option<Dem1m>> = std::sync::OnceLock::new();
    D.get_or_init(|| {
        let root = Root::discover()?;
        Dem1m::open(&root).map_err(|e| eprintln!("terrain detail: the 1 m DEM is not used ({e:#}); the 8 m terrain as before")).ok()
    })
    .as_ref()
}

/// (`--no-city-relief`, Docs/HILLS-PLAN.md stage H3) The hill builder's cooked relief over
/// the whole city (eo-hills `relief`, `<cache>/hills/relief-v1.bin`: the 1 m DEM's high-pass
/// at the patches' 2 m vertices, by the same definition as `build`'s, kept where it passes
/// 0.1 m), opened once (None: switched off, missing or refused: the patches round the photo
/// eyes alone, from the survey, as before).
pub fn cooked() -> Option<&'static eo_hills::relief::Relief> {
    static R: std::sync::OnceLock<Option<eo_hills::relief::Relief>> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        if std::env::args().any(|a| a == "--no-city-relief") {
            return None;
        }
        let p = eo_hills::relief::file_path(&eo_hills::cache_dir());
        if !p.is_file() {
            eprintln!("terrain detail: no city relief at {} (eo-hills cook relief); the photo eyes' patches alone", p.display());
            return None;
        }
        eo_hills::relief::Relief::open(&p).map_err(|e| eprintln!("terrain detail: the city relief is not used ({e:#})")).ok()
    })
    .as_ref()
}

/// A cooked block more than this share of whose points lie inside footprints is left to the
/// tiles (backyards and light wells between buildings, whose relief is the buildings' edges).
pub const COOKED_BUILT_MAX: f32 = 0.2;
/// ... and one with less relief than this (m, before the height scale): the cook keeps 92 % of
/// the land at its 0.1 m (house pads, terraced yards, street cuts), 35,601 blocks; the hills
/// and wild parks are a few thousand.
pub const COOKED_RELIEF_MIN: f32 = 0.5;
/// ... and one whose 4 x 4 sample points are less than this share open ground or mapped
/// greens (`OPEN_TONES` and the parks' "grass").
pub const COOKED_GREEN_MIN: f64 = 0.5;

/// The cooked relief at patch `key`'s vertex (i, j) (0..=32 inside it; -1 and 33 reach into
/// the neighbours, for the normals), metres before the height scale (None: no block there).
fn cooked_at(r: &eo_hills::relief::Relief, key: (i64, i64), i: i64, j: i64) -> Option<f64> {
    let last = PATCH_CELLS * STEPS; // 32
    let (bx, ii) = if i < 0 { (key.0 - 1, i + last) } else if i > last { (key.0 + 1, i - last) } else { (key.0, i) };
    let (bz, jj) = if j < 0 { (key.1 - 1, j + last) } else if j > last { (key.1 + 1, j - last) } else { (key.1, j) };
    r.block(bx, bz).map(|b| b[(jj * (last + 1) + ii) as usize] as f64 * 0.01)
}

/// The survey's NAIP mosaic (None: no survey root).
pub fn naip() -> Option<&'static Naip> {
    static N: std::sync::OnceLock<Option<Naip>> = std::sync::OnceLock::new();
    N.get_or_init(|| Root::discover().map(|r| Naip::open(&r))).as_ref()
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn point_in_tri(p: [f64; 2], t: &[[f32; 2]; 3]) -> bool {
    let s = |a: [f32; 2], b: [f32; 2]| (b[0] as f64 - a[0] as f64) * (p[1] - a[1] as f64) - (b[1] as f64 - a[1] as f64) * (p[0] - a[0] as f64);
    let (d0, d1, d2) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
    !((d0 < 0.0 || d1 < 0.0 || d2 < 0.0) && (d0 > 0.0 || d1 > 0.0 || d2 > 0.0))
}

fn point_in_ring(p: [f64; 2], r: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let n = r.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (r[i], r[j]);
        if (a[1] as f64 > p[1]) != (b[1] as f64 > p[1]) && p[0] < (b[0] as f64 - a[0] as f64) * (p[1] - a[1] as f64) / (b[1] as f64 - a[1] as f64) + a[0] as f64 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Two-pass 3-4 chamfer distance (metres) to the true cells of an n x n raster of `step`.
fn chamfer(cells: &[bool], n: usize, step: f64) -> Vec<f32> {
    let mut dist: Vec<f32> = cells.iter().map(|&c| if c { 0.0 } else { f32::MAX }).collect();
    let (a, b) = (step as f32, (step * std::f64::consts::SQRT_2) as f32);
    for j in 0..n {
        for i in 0..n {
            let k = j * n + i;
            let mut d = dist[k];
            if i > 0 {
                d = d.min(dist[k - 1] + a);
            }
            if j > 0 {
                d = d.min(dist[k - n] + a);
                if i > 0 {
                    d = d.min(dist[k - n - 1] + b);
                }
                if i + 1 < n {
                    d = d.min(dist[k - n + 1] + b);
                }
            }
            dist[k] = d;
        }
    }
    for j in (0..n).rev() {
        for i in (0..n).rev() {
            let k = j * n + i;
            let mut d = dist[k];
            if i + 1 < n {
                d = d.min(dist[k + 1] + a);
            }
            if j + 1 < n {
                d = d.min(dist[k + n] + a);
                if i + 1 < n {
                    d = d.min(dist[k + n + 1] + b);
                }
                if i > 0 {
                    d = d.min(dist[k + n - 1] + b);
                }
            }
            dist[k] = d;
        }
    }
    dist
}

/// Build the relief patches (None: no DEM, or no patch inside the grid).
pub fn build(inp: &Inputs, materials: Option<&Materials<'_>>) -> Option<Detail> {
    let t0 = std::time::Instant::now();
    let dem = dem();
    let city = cooked();
    if dem.is_none() && city.is_none() {
        return None;
    }
    let l = inp.lattice;
    // the photo eyes' patches (from the survey), and (`--no-city-relief`) every cooked block
    // over open ground
    let mut keys = if dem.is_some() { patch_keys(&l) } else { vec![] };
    if let Some(r) = city {
        let have: HashSet<(i64, i64)> = keys.iter().copied().collect();
        let (pw, ph) = (l.width / PATCH_CELLS, l.height / PATCH_CELLS);
        let cover = eo_landmarks::ground_cover::installed();
        let pm = PATCH_CELLS as f64 * l.cell;
        let green = |k: &(i64, i64)| -> f64 {
            let (x0, z0) = (l.vx(k.0 * PATCH_CELLS), l.vz(k.1 * PATCH_CELLS));
            let hit = (0..16).filter(|s| cover.and_then(|c| c.tone_name_at(x0 + ((s % 4) as f64 + 0.5) * pm / 4.0, z0 + ((s / 4) as f64 + 0.5) * pm / 4.0)).is_some_and(|n| OPEN_TONES.contains(&n) || n == "grass")).count();
            hit as f64 / 16.0
        };
        // (`--no-deck-ground`) no relief over a roof deck brought down to the streets' grade: its
        // cooked relief is the deck's walls over the survey's ground (eo-data `decks`)
        let mut on_deck: HashSet<(i64, i64)> = HashSet::new();
        for (i, j) in eo_data::decks::table().into_iter().flat_map(|t| t.cells()) {
            let (i, j) = (i as i64, j as i64);
            for ki in [i / PATCH_CELLS, (i - 1).div_euclid(PATCH_CELLS)] {
                for kj in [j / PATCH_CELLS, (j - 1).div_euclid(PATCH_CELLS)] {
                    on_deck.insert((ki, kj));
                }
            }
        }
        let mut more: Vec<(i64, i64)> = r
            .keys()
            .filter(|k| !have.contains(k) && !on_deck.contains(k) && k.0 >= 0 && k.1 >= 0 && k.0 < pw && k.1 < ph && r.info(k.0, k.1).is_some_and(|b| b.built_share <= COOKED_BUILT_MAX && b.max_abs_m >= COOKED_RELIEF_MIN) && green(k) >= COOKED_GREEN_MIN)
            .collect();
        more.sort();
        keys.extend(more);
    }
    if keys.is_empty() {
        return None;
    }
    let keyset: HashSet<(i64, i64)> = keys.iter().copied().collect();
    let step = l.cell / STEPS as f64;
    // the DEM (real metres) at a game point, averaged over a square of half-side `half` (4 taps)
    let dem_at = |x: f64, z: f64, half: f64| -> Option<f64> {
        let mut acc = 0.0;
        for (dx, dz) in [(-half, -half), (half, -half), (-half, half), (half, half)] {
            let (lat, lon) = inp.frame.lat_lon(x + dx, z + dz);
            acc += dem?.height(lon, lat)? as f64;
        }
        Some(acc / 4.0)
    };
    // the 8 m low-pass at grid vertex (cx, cz): the DEM averaged over the cell round it (16 x 4 taps)
    let low = |cx: i64, cz: i64| -> Option<f64> {
        let (x, z) = (l.vx(cx), l.vz(cz));
        let mut acc = 0.0;
        for j in 0..4 {
            for i in 0..4 {
                let (dx, dz) = ((i as f64 - 1.5) * l.cell / 4.0, (j as f64 - 1.5) * l.cell / 4.0);
                acc += dem_at(x + dx, z + dz, l.cell / 16.0)?;
            }
        }
        Some(acc / 16.0)
    };
    // the street triangles and footprints binned by the patches they reach (one cell of margin)
    let pm = PATCH_CELLS as f64 * l.cell;
    let mut tri_bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut ring_bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let bin = |lo: [f64; 2], hi: [f64; 2], k: usize, bins: &mut HashMap<(i64, i64), Vec<usize>>| {
        let pa = (((lo[0] - l.cell - l.vx(0)) / pm).floor() as i64, ((lo[1] - l.cell - l.vz(0)) / pm).floor() as i64);
        let pb = (((hi[0] + l.cell - l.vx(0)) / pm).floor() as i64, ((hi[1] + l.cell - l.vz(0)) / pm).floor() as i64);
        if (pb.0 - pa.0) * (pb.1 - pa.1) > 4096 {
            return;
        }
        for pz in pa.1..=pb.1 {
            for px in pa.0..=pb.0 {
                if keyset.contains(&(px, pz)) {
                    bins.entry((px, pz)).or_default().push(k);
                }
            }
        }
    };
    for (k, t) in inp.street_tris.iter().enumerate() {
        let lo = [t.iter().map(|p| p[0]).fold(f32::MAX, f32::min) as f64, t.iter().map(|p| p[1]).fold(f32::MAX, f32::min) as f64];
        let hi = [t.iter().map(|p| p[0]).fold(f32::MIN, f32::max) as f64, t.iter().map(|p| p[1]).fold(f32::MIN, f32::max) as f64];
        bin(lo, hi, k, &mut tri_bins);
    }
    for (k, r) in inp.buildings.iter().enumerate() {
        if r.is_empty() {
            continue;
        }
        let lo = [r.iter().map(|p| p[0]).fold(f32::MAX, f32::min) as f64, r.iter().map(|p| p[1]).fold(f32::MAX, f32::min) as f64];
        let hi = [r.iter().map(|p| p[0]).fold(f32::MIN, f32::max) as f64, r.iter().map(|p| p[1]).fold(f32::MIN, f32::max) as f64];
        bin(lo, hi, k, &mut ring_bins);
    }
    let cover = eo_landmarks::ground_cover::installed();
    // `keyset`: the patches drawn (an outer edge is one with no patch beyond)
    let build_patch = |key: (i64, i64), keyset: &HashSet<(i64, i64)>| -> Patch {
        // a patch with a cooked block takes its relief from it (the photo eyes' patches from the
        // survey where it is present and holds none)
        let from_city = city.is_some_and(|r| r.block(key.0, key.1).is_some()) && !(dem.is_some() && REGIONS.iter().any(|(_, e)| near_region(&l, key, e)));
        let (cx0, cz0) = (key.0 * PATCH_CELLS, key.1 * PATCH_CELLS);
        let (x0, z0) = (l.vx(cx0), l.vz(cz0));
        let span = PATCH_CELLS as f64 * l.cell;
        // the tiles' vertex heights round the patch (two cells of margin), read once
        let cw = (PATCH_CELLS + 5) as usize;
        let hc: Vec<f32> = (0..cw * cw).map(|k| (inp.vertex_height)(cx0 - 2 + (k % cw) as i64, cz0 - 2 + (k / cw) as i64)).collect();
        // the carpet lifts all open ground by one constant (eo-street `openGroundLiftM`, about
        // 0.1 m): pressed is a move away from the patch's own (median) offset, so the lift alone
        // presses nothing (with it, every open hillside read as pressed and took no relief)
        let offset: Vec<f32> = (0..cw * cw)
            .map(|k| {
                let (cx, cz) = (cx0 - 2 + (k % cw) as i64, cz0 - 2 + (k / cw) as i64);
                hc[k] - (inp.raw_ground)(l.vx(cx), l.vz(cz))
            })
            .collect();
        let lift = {
            let mut o = offset.clone();
            o.sort_by(|a, b| a.total_cmp(b));
            o[o.len() / 2]
        };
        let pressed: Vec<bool> = offset.iter().map(|d| (d - lift).abs() > PRESSED_M).collect();
        let vh = |cx: i64, cz: i64| hc[((cz - cz0 + 2).clamp(0, cw as i64 - 1) as usize) * cw + (cx - cx0 + 2).clamp(0, cw as i64 - 1) as usize];
        let base_at = |x: f64, z: f64| -> f32 {
            let fx = (x - l.vx(0)) / l.cell;
            let fz = (z - l.vz(0)) / l.cell;
            let (qx, qz) = (fx.floor() as i64, fz.floor() as i64);
            let (u, v) = ((fx - qx as f64) as f32, (fz - qz as f64) as f32);
            cell_tri(vh(qx, qz), vh(qx + 1, qz), vh(qx, qz + 1), vh(qx + 1, qz + 1), u, v)
        };
        // the mask over the patch and a margin
        let margin = MASK_FADE_M + step;
        let n = ((span + 2.0 * margin) / step).ceil() as usize;
        let (mx0, mz0) = (x0 - margin, z0 - margin);
                let tris: Vec<&[[f32; 2]; 3]> = tri_bins.get(&key).map(|v| v.iter().map(|&k| &inp.street_tris[k]).collect()).unwrap_or_default();
        let rings: Vec<&[[f32; 2]]> = ring_bins.get(&key).map(|v| v.iter().map(|&k| inp.buildings[k]).collect()).unwrap_or_default();
        let mut cells = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let p = [mx0 + (i as f64 + 0.5) * step, mz0 + (j as f64 + 0.5) * step];
                let mut m = tris.iter().any(|t| point_in_tri(p, t)) || rings.iter().any(|r| point_in_ring(p, r));
                // round a photo eye (`EYE_CLEAR_M`)
                m = m || REGIONS.iter().any(|(_, e)| (p[0] - e[0]).hypot(p[1] - e[1]) < EYE_CLEAR_M);
                // the mapped lawns and parks: every ground-cover tone that is not open ground
                // (a cooked block's own relief also reaches the mapped greens: Bernal Heights,
                // Twin Peaks and the city's other hills are mapped as parks; a mown lawn's
                // relief is about nothing anyway)
                m = m || cover.and_then(|c| c.tone_name_at(p[0], p[1])).is_some_and(|n| !OPEN_TONES.contains(&n) && !(from_city && n == "grass"));
                if !m {
                    // pressed: a corner of the grid cell under it moved by the carpet or the press
                    let fx = ((p[0] - l.vx(0)) / l.cell).floor() as i64;
                    let fz = ((p[1] - l.vz(0)) / l.cell).floor() as i64;
                    m = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().any(|&(a, b)| pressed[((fz + b - cz0 + 2).clamp(0, cw as i64 - 1) as usize) * cw + (fx + a - cx0 + 2).clamp(0, cw as i64 - 1) as usize])
                        || base_at(p[0], p[1]) < inp.sea_y + 0.5
                        || pale_ground(&inp.frame, p[0], p[1], step) > 0.5
                        || ((inp.colour)(p[0] as f32, p[1] as f32) >> 24) < MATERIAL_ALPHA;
                }
                cells[j * n + i] = m;
            }
        }
        let dist = chamfer(&cells, n, step);
        let dist_at = |x: f64, z: f64| -> f64 {
            let i = (((x - mx0) / step).floor() as i64).clamp(0, n as i64 - 1) as usize;
            let j = (((z - mz0) / step).floor() as i64).clamp(0, n as i64 - 1) as usize;
            dist[j * n + i] as f64
        };
        // the distance to the outer edge: to each side or corner with no patch beyond
        let missing = |dx: i64, dz: i64| !keyset.contains(&(key.0 + dx, key.1 + dz));
        let edge_d = |x: f64, z: f64| -> f64 {
            let (u, v) = (x - x0, z - z0);
            let mut d = f64::MAX;
            for (dx, dz, e) in [(-1, 0, u), (1, 0, span - u), (0, -1, v), (0, 1, span - v)] {
                if missing(dx, dz) {
                    d = d.min(e);
                }
            }
            for (dx, dz, cu, cv) in [(-1, -1, 0.0, 0.0), (1, -1, span, 0.0), (-1, 1, 0.0, span), (1, 1, span, span)] {
                if missing(dx, dz) {
                    d = d.min(((u - cu).powi(2) + (v - cv).powi(2)).sqrt());
                }
            }
            d.max(0.0)
        };
        // the low-pass at the patch's grid vertices and one ring
        let lw = (PATCH_CELLS + 3) as usize;
        let lp: Vec<Option<f64>> = (0..lw * lw).map(|k| low(cx0 - 1 + (k % lw) as i64, cz0 - 1 + (k / lw) as i64)).collect();
        let lp_at = |cx: i64, cz: i64| lp[((cz - cz0 + 1).clamp(0, lw as i64 - 1) as usize) * lw + (cx - cx0 + 1).clamp(0, lw as i64 - 1) as usize];
        // heights on the patch's vertices plus one ring (for the normals)
        let hw = SIDE + 2;
        let mut hs = vec![0.0f32; hw * hw];
        let mut ad = vec![0.0f32; hw * hw];
        // the second flora pass's knoll (`--no-knoll-ground`): (top, face) at each vertex
        let mut kn = vec![(0.0f64, 0.0f64); hw * hw];
        let (mut relieved, mut max_relief) = (0usize, 0.0f32);
        for jj in 0..hw {
            for ii in 0..hw {
                let (i, j) = (ii as i64 - 1, jj as i64 - 1);
                let (x, z) = (x0 + i as f64 * step, z0 + j as f64 * step);
                let base = base_at(x, z);
                let (qx, qz) = (cx0 + i.div_euclid(STEPS), cz0 + j.div_euclid(STEPS));
                let (u, v) = (i.rem_euclid(STEPS) as f32 / STEPS as f32, j.rem_euclid(STEPS) as f32 / STEPS as f32);
                let mut add = 0.0f32;
                let w = || smooth(0.0, MASK_FADE_M, dist_at(x, z) - step * 0.5) * smooth(0.0, EDGE_FADE_M, edge_d(x, z));
                if let Some(rv) = city.filter(|_| from_city).and_then(|r| cooked_at(r, key, i, j)) {
                    // (`--no-city-relief`) the cooked high-pass, by the same definition as below
                    add = ((rv * inp.height_scale) as f32).clamp(-MAX_RELIEF_M, MAX_RELIEF_M) * w() as f32;
                } else if let ((Some(a), Some(b), Some(c), Some(d)), Some(dm)) = ((lp_at(qx, qz), lp_at(qx + 1, qz), lp_at(qx, qz + 1), lp_at(qx + 1, qz + 1)), dem_at(x, z, step / 4.0)) {
                    let lpv = cell_tri(a as f32, b as f32, c as f32, d as f32, u, v) as f64;
                    add = (((dm - lpv) * inp.height_scale) as f32).clamp(-MAX_RELIEF_M, MAX_RELIEF_M) * w() as f32;
                }
                if let Some(dem) = dem.filter(|_| materials.is_some() && eo_landmarks::flora2::flora2().knoll && base as f64 > eo_landmarks::flora2::KNOLL_MIN_Y) {
                    if let Some((t, f, down)) = knoll_at(inp, dem, x, z) {
                        kn[jj * hw + ii] = (t, f);
                        if eo_landmarks::flora2::flora2().relief && f > 0.0 {
                            // the cut face's gullies, under the same masks as the relief
                            add += (eo_landmarks::flora2::gully_m(x, z, down, f) * w()) as f32;
                        }
                    }
                }
                hs[jj * hw + ii] = base + add;
                ad[jj * hw + ii] = add;
                if (1..=SIDE).contains(&ii) && (1..=SIDE).contains(&jj) {
                    if add.abs() > RELIEF_EPS_M {
                        relieved += 1;
                    }
                    max_relief = max_relief.max(add.abs());
                }
            }
        }
        // the tiles' vertex normals (scene.rs: central differences over two cells, normalised),
        // which the rasteriser interpolates over the 8 m triangles
        let tile_n = |cx: i64, cz: i64| -> glam::Vec3 {
            let dx = (vh(cx + 1, cz) - vh(cx - 1, cz)) / (2.0 * l.cell as f32);
            let dz = (vh(cx, cz + 1) - vh(cx, cz - 1)) / (2.0 * l.cell as f32);
            glam::Vec3::new(-dx, 1.0, -dz).normalize()
        };
        let tn: Vec<glam::Vec3> = (0..lw * lw).map(|k| tile_n(cx0 - 1 + (k % lw) as i64, cz0 - 1 + (k / lw) as i64)).collect();
        let tn_at = |cx: i64, cz: i64| tn[((cz - cz0 + 1).clamp(0, lw as i64 - 1) as usize) * lw + (cx - cx0 + 1).clamp(0, lw as i64 - 1) as usize];
        let mut verts = Vec::with_capacity(SIDE * SIDE);
        let mut relief = Vec::with_capacity(SIDE * SIDE);
        let mut material = 0usize;
        for j in 0..SIDE {
            for i in 0..SIDE {
                let k = (j + 1) * hw + i + 1;
                let (x, z) = (x0 + i as f64 * step, z0 + j as f64 * step);
                // the 8 m surface's shading normal here (as the tiles draw it), bent by the relief's gradient
                let (qx, qz) = (cx0 + (i as i64).div_euclid(STEPS), cz0 + (j as i64).div_euclid(STEPS));
                let (u, v) = ((i as i64).rem_euclid(STEPS) as f32 / STEPS as f32, (j as i64).rem_euclid(STEPS) as f32 / STEPS as f32);
                let (a, b, c, d) = (tn_at(qx, qz), tn_at(qx + 1, qz), tn_at(qx, qz + 1), tn_at(qx + 1, qz + 1));
                let nb = glam::Vec3::new(cell_tri(a.x, b.x, c.x, d.x, u, v), cell_tri(a.y, b.y, c.y, d.y, u, v), cell_tri(a.z, b.z, c.z, d.z, u, v));
                let (gx, gz) = (-nb.x / nb.y.max(1e-3), -nb.z / nb.y.max(1e-3));
                let rx = (ad[k + 1] - ad[k - 1]) / (2.0 * step as f32);
                let rz = (ad[k + hw] - ad[k - hw]) / (2.0 * step as f32);
                let nrm = glam::Vec3::new(-(gx + rx), 1.0, -(gz + rz)).normalize();
                let mut color = (inp.colour)(x as f32, z as f32);
                let mut normal = eo_assets::glb::pack_normal(nrm);
                if let Some(m) = materials {
                    let (c, w) = m.material(x, z, color, nrm.y as f64, hs[k], step);
                    let (c, w) = knoll_material(c, w, kn[k]);
                    color = c;
                    normal |= (w as u8 as u32) << 24;
                }
                if (color >> 24) == MATERIAL_ALPHA {
                    material += 1;
                }
                if let Some(w) = sand_slope_byte(x, z, color) {
                    normal |= (w as u8 as u32) << 24;
                }
                verts.push(Vertex { pos: [x as f32, hs[k], z as f32], normal, color });
                relief.push(ad[k]);
            }
        }
        Patch { key, verts, relief, relieved, max_relief_m: max_relief, material }
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let build_all = |keys: &[(i64, i64)], set: &HashSet<(i64, i64)>| -> Vec<Patch> {
        let chunk = keys.len().div_ceil(threads * 4).max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let chunks: Vec<&[(i64, i64)]> = keys.chunks(chunk).collect();
        std::thread::scope(|s| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let mut out = vec![];
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(c) = chunks.get(i) else { break };
                            out.extend(c.iter().map(|&k| build_patch(k, set)));
                        }
                        out
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    };
    let mut patches = build_all(&keys, &keyset);
    // A patch with no relief and no survey material draws the 8 m tiles' surface exactly (the
    // sea, sand, paving, lawns): it is left to the tiles. Its neighbours were built with it
    // present (no edge fade toward it), so each one beside a dropped patch is rebuilt with the
    // kept set: its edge toward the dropped one is then an outer edge, on the 8 m surface.
    let total = patches.len();
    patches.retain(|p| p.relieved > 0 || p.material > 0);
    let kept: HashSet<(i64, i64)> = patches.iter().map(|p| p.key).collect();
    let near_dropped = |k: &(i64, i64)| (-1..=1).any(|dz| (-1..=1).any(|dx| (dx, dz) != (0, 0) && keyset.contains(&(k.0 + dx, k.1 + dz)) && !kept.contains(&(k.0 + dx, k.1 + dz))));
    let redo: Vec<(i64, i64)> = patches.iter().map(|p| p.key).filter(|k| near_dropped(k)).collect();
    let rebuilt = redo.len();
    if !redo.is_empty() {
        let mut fresh: HashMap<(i64, i64), Patch> = build_all(&redo, &kept).into_iter().map(|p| (p.key, p)).collect();
        for p in patches.iter_mut() {
            if let Some(q) = fresh.remove(&p.key) {
                *p = q;
            }
        }
    }
    patches.sort_by_key(|p| p.key);
    let stats = DetailStats {
        patches: patches.len(),
        dropped: total - patches.len(),
        rebuilt,
        vertices: patches.len() * SIDE * SIDE,
        relieved: patches.iter().map(|p| p.relieved).sum(),
        max_relief_m: patches.iter().map(|p| p.max_relief_m).fold(0.0, f32::max),
        ms: t0.elapsed().as_secs_f64() * 1e3,
    };
    let index = patches.iter().enumerate().map(|(i, p)| (p.key, i)).collect();
    let d = Detail { patches, keys: kept, lattice: l, stats, index };
    // EO_TERRAIN_DETAIL_PROBE="x,z,dx,dz": the drawn base, the relief and the DEM along a line (diagnostic)
    if let Some(v) = std::env::var("EO_TERRAIN_DETAIL_PROBE").ok() {
        let f: Vec<f64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if f.len() == 4 {
            for k in 0..40 {
                let (x, z) = (f[0] + f[2] * k as f64 * 3.0, f[1] + f[3] * k as f64 * 3.0);
                let fx = (x - l.vx(0)) / l.cell;
                let fz = (z - l.vz(0)) / l.cell;
                let (qx, qz) = (fx.floor() as i64, fz.floor() as i64);
                let base = drawn_base_q(inp, qx, qz, (fx - qx as f64) as f32, (fz - qz as f64) as f32);
                let (lat, lon) = inp.frame.lat_lon(x, z);
                let dh = dem.and_then(|dm| dm.height(lon, lat));
                let kn = dem.and_then(|dm| knoll_at(inp, dm, x, z));
                eprintln!("probe {:6.1} m: base {:8.2} drawn {:?} dem {:?} raw {:.2} knoll {:?} gully {:?} relief {:?}", k as f64 * 3.0, base, d.height(x, z), dh, (inp.raw_ground)(x, z), kn, kn.map(|(_, f, dn)| eo_landmarks::flora2::gully_m(x, z, dn, f)), d.relief_height(x, z));
            }
        }
    }
    Some(d)
}

fn drawn_base_q(inp: &Inputs, qx: i64, qz: i64, u: f32, v: f32) -> f32 {
    let h = inp.vertex_height;
    cell_tri(h(qx, qz), h(qx + 1, qz), h(qx, qz + 1), h(qx + 1, qz + 1), u, v)
}

// ---- sand ----------------------------------------------------------------------

/// The steepest foreshore the sand vertex's slope byte carries (rise over
/// run; eo-water `beaches::SLOPE_RANGE`'s steep end, 1:4).
pub const SAND_SLOPE_FULL: f64 = 0.25;

/// A beach-sand vertex's foreshore slope for the shader's damp band, wrack
/// and prints (`--no-sand-detail2`; shore_surface.wgsl `gg2_foreshore_h`):
/// each beach's measured slope (eo-water `beaches`, the slope the carpet
/// lays that beach at; Crissy Field about 1:10, Baker Beach about 1:18) in
/// the normal's w byte, as a share of `SAND_SLOPE_FULL`. None off the sand,
/// with the beach profiles off, or before they are measured.
pub fn sand_slope_byte(x: f64, z: f64, rgba: u32) -> Option<i8> {
    if (rgba >> 24) >= MATERIAL_ALPHA || !eo_landmarks::features::gg3_ground("sand-detail2") || !eo_water::switches::is_on(eo_water::switches::BEACH_PROFILES) {
        return None;
    }
    let v = eo_water::beaches::installed()?;
    let slope = eo_water::beaches::slope_at(v, x, z, eo_water::field::BEACH_FORESHORE_SLOPE);
    Some(((slope / SAND_SLOPE_FULL).clamp(0.0, 1.0) * 127.0).round().max(1.0) as i8)
}

// ---- materials -----------------------------------------------------------------

/// The ground's materials from the survey (`--no-terrain-materials`), at a
/// vertex: over the NAIP extent, on open ground (the far land, the mapped
/// wild grass, dry grass, coastal scrub, understorey and bare ground, and
/// unmapped ground WorldCover calls trees, shrubland, grassland, bare or moss;
/// never the built-up yards, the mapped lawns, paving, courts or sand), the
/// vertex carries:
///
/// * the substrate's colour in its RGB (alpha 254 marks it): bare soil, mixed
///   toward the coastal scrub's albedo (the far layer's `FAR_SCRUB`) by the
///   canopy's share, the planted shrubs standing on it;
/// * the grass cover share in its normal's w byte (0..127), which the shader
///   draws in the season's grass colour (`look.tone2.w`, eo_look::season) with
///   the grass scans; rock on the steep faces is the shader's, from the slope.
///
/// Method (stated, not fitted to the frames). The cover's kind comes from
/// the land cover first (ESA WorldCover 2021, the mapped ground-cover tone
/// where one is mapped), the NAIP refining it, because the NAIP's summer
/// image cannot tell cured grass from bare soil: over the whole Marin extent
/// of the NAIP (40,000 points away from the headland eye's square) the
/// WorldCover grassland's Excess Green reads p25 0.011, p50 0.041, p75 0.092,
/// and the eroded faces at the headland 0.02-0.04 (the
/// `the_marin_cover_follows_the_land_cover` test prints the histograms).
///
/// * Grassland (WorldCover 30, moss 100, unmapped 0; the mapped wild and dry
///   grass): grass, but for the shrubs in it (NAIP vegetation darker than
///   `CANOPY_BRIGHT`), pale ground (`PALE_*`: gravel, paths, slabs) and faces
///   too steep to hold a sward (`SWARD_SLOPE`), where the soil shows.
/// * Trees and shrubland (WorldCover 10, 20; the mapped scrub and
///   understorey): coastal scrub, its canopy the NAIP's vegetation share,
///   the openings between the crowns grass (as on the open slopes), the same
///   pale and steep exceptions.
/// * Bare (WorldCover 60; the mapped bare earth and rock): soil, with grass
///   where the NAIP shows green herbs (vegetation lighter than the canopy).
///
/// The NAIP's vegetation share over the vertex's footprint (5 x 5 taps) is
/// the Excess Green index ExG = 2g - r - b of the chromatic coordinates
/// (Woebbecke et al. 1995, Trans. ASAE 38(1)) ramped from `EXG_BARE` to
/// `EXG_FULL` (INFERRED: the NAIP's eroded faces and the battery's concrete
/// read 0.00-0.04, its scrub and the Presidio forest 0.07-0.14, sampled away
/// from the photo views' frames). Vegetation darker than `CANOPY_BRIGHT` (the
/// mean of the three channels, 0-255) is shrub canopy (coastal scrub's crowns
/// shade the ground; the open sward is lighter: the NAIP's headland scrub
/// reads 82-86, its grass 108-122).
///
/// The substrate. North of the strait (the Marin Headlands' Franciscan chert
/// and greenstone) the soil is the Tamalpais series, whose type location is
/// in the Marin Headlands (USDA NRCS Official Series Description, TAMALPAIS,
/// rev. 03/2003): an eroded face shows its B2t horizon, dry 5YR 4/4; the
/// surface its A horizon, dry 5YR 5/4 (from bare at 0.5 to eroded at 0.8 rise
/// over run, `ERODED_SLOPE`). Munsell to sRGB through the renotation (each
/// tone's luminance is its Munsell value's: 4 -> Y 0.12, 5 -> 0.20).
/// South of it the substrate is the ground's round-2 tone (bare earth, the
/// cliff tone), which no cited soil colour replaces yet.
pub struct Materials<'a> {
    pub frame: GeoFrame,
    pub land: Option<&'a eo_data::LandCover>,
    pub sea_y: f32,
}

pub const EXG_BARE: f64 = 0.02;
pub const EXG_FULL: f64 = 0.08;
pub const CANOPY_BRIGHT: [f64; 2] = [90.0, 105.0];
/// Pale, yellow-grey and bright bare ground (gravel, paths, the batteries'
/// slabs: the NAIP's mean over 140, the battery pad's 148): the ground keeps
/// its own tone. INFERRED from NAIP samples at the headland.
pub const PALE_BRIGHT: [f64; 2] = [135.0, 150.0];
pub const PALE_YELLOW: [f64; 2] = [0.04, 0.10];
/// Tamalpais series (NRCS OSD), Munsell to sRGB: B2t dry 5YR 4/4, A dry 5YR 5/4.
pub const SOIL_ERODED_SRGB: [u8; 3] = [126, 90, 68];
pub const SOIL_SURFACE_SRGB: [u8; 3] = [153, 115, 87];
/// North of this latitude the ground is the Marin Headlands' (the strait's mid-line under the bridge).
pub const MARIN_LAT: f64 = 37.8135;
/// The slope (rise over run) past which the soil is an eroded face (its B
/// horizon), INFERRED: about 35 degrees, the angle of repose of loose soil.
pub const ERODED_SLOPE: [f64; 2] = [0.5, 0.8];
/// The slope (rise over run) over which a sward gives way to the eroded
/// face: from 0.8 (39 degrees, past the angle of repose, `ERODED_SLOPE`'s
/// end) to 1.2 (50 degrees, where the shader's rock is half drawn).
/// INFERRED: a sward needs a soil the slope holds.
pub const SWARD_SLOPE: [f64; 2] = [0.8, 1.2];
/// The alpha byte of a vertex the terrain materials draw.
pub const MATERIAL_ALPHA: u32 = 254;

/// The mapped ground-cover tones that are open ground.
pub const OPEN_TONES: [&str; 6] = ["wildGrass", "dryGrass", "wildScrub", "understory", "bareEarth", "bareRock"];

/// What the land cover says a vertex's open ground is (`Materials`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prior {
    Grass,
    Scrub,
    Bare,
}

/// The prior from the mapped tone, else the WorldCover class.
pub fn prior(tone: Option<&str>, class: u8) -> Prior {
    match tone {
        Some("wildGrass") | Some("dryGrass") => Prior::Grass,
        Some("wildScrub") | Some("understory") => Prior::Scrub,
        Some("bareEarth") | Some("bareRock") => Prior::Bare,
        _ => match class {
            10 | 20 => Prior::Scrub,
            60 => Prior::Bare,
            _ => Prior::Grass,
        },
    }
}

/// (grass share, canopy share) from the prior, the NAIP's mean vegetation
/// share `veg`, its dark (canopy) part `dark_veg`, its pale share and the
/// slope (rise over run): `Materials`' rules.
pub fn cover_shares(prior: Prior, veg: f64, dark_veg: f64, pale: f64, slope: f64) -> (f64, f64) {
    let sward = 1.0 - smooth(SWARD_SLOPE[0], SWARD_SLOPE[1], slope);
    let (grass, canopy) = match prior {
        Prior::Grass => ((1.0 - dark_veg - pale).max(0.0), dark_veg),
        Prior::Scrub => ((1.0 - veg - pale).max(0.0), veg),
        Prior::Bare => ((veg - dark_veg).max(0.0), dark_veg),
    };
    ((grass * sward).clamp(0.0, 1.0), canopy.clamp(0.0, 1.0))
}

impl Materials<'_> {
    /// (colour, grass share byte) at a vertex of drawn height `y` whose normal's
    /// y is `ny`, over a footprint of `fp` metres; unchanged off open ground.
    pub fn material(&self, x: f64, z: f64, rgba: u32, ny: f64, y: f32, fp: f64) -> (u32, i8) {
        if (rgba >> 24) != 255 || y < self.sea_y + 1.0 {
            return (rgba, 0);
        }
        let Some(naip) = naip() else { return (rgba, 0) };
        let class = self.land.map(|l| l.class_at(x, z)).unwrap_or(0);
        let tone = eo_landmarks::ground_cover::installed().and_then(|c| c.tone_name_at(x, z));
        let open = match tone {
            Some(n) => OPEN_TONES.contains(&n),
            None => matches!(class, 10 | 20 | 30 | 60 | 100),
        };
        if !open || class == 50 {
            return (rgba, 0);
        }
        let Some((veg, dark_veg, pale)) = naip_shares(naip, &self.frame, x, z, fp) else { return (rgba, 0) };
        let slope = (1.0 - ny * ny).max(0.0).sqrt() / ny.max(1e-3);
        let enc = |v: f64| (v.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u32;
        // the second flora pass (`--no-bluff-rock`): the Baker bluff's steep faces are
        // serpentinite rock (eo_landmarks::flora2 `bluff_albedo`), pale ground in the image
        // too (the bluff's slides and its Colma skin: the rock stands under them), the
        // shader's bluff code in the grass byte (-64 to -127: a sheared, scaly rock with
        // grass on its ledges).
        let (lat, lon) = self.frame.lat_lon(x, z);
        let face = if lat <= MARIN_LAT && eo_landmarks::flora2::flora2().bluff_rock { eo_landmarks::flora2::bluff_face(lon, lat, slope.atan().to_degrees()) } else { 0.0 };
        if face > 0.02 {
            let own = [0, 8, 16].map(|s| srgb_lin(((rgba >> s) & 255) as u8));
            let substrate = mix3(own, crate::farshore::FAR_SCRUB.map(|v| v as f64), dark_veg);
            let rock = eo_landmarks::flora2::bluff_albedo(x, z, y as f64 - eo_data::SEA_LEVEL_Y);
            let m = mix3(substrate, rock, face * (1.0 - 0.5 * dark_veg));
            let c = enc(m[0]) | (enc(m[1]) << 8) | (enc(m[2]) << 16) | (MATERIAL_ALPHA << 24);
            return (c, -(64 + (63.0 * face).round() as i8));
        }
        if pale > 0.5 {
            return (rgba, 0); // gravel, a path, a slab: the ground's own tone
        }
        let mut pr = prior(tone, class);
        // the second grass pass's continuous meadow (`--no-meadow-cover`;
        // eo_landmarks::vegetation::meadow_ground): bare earth and the bare class in
        // the NAIP image carry the meadow patches, so the ground under them is the
        // sward's too, not soil (mapped bare rock stays rock).
        // The third pass (`--no-bare-ground`) keeps the eroded bare class bare: a steep
        // face (past `BARE_FACE_SLOPE`) or red-brown soil in the NAIP (`soil_share`)
        // as eo_landmarks::vegetation::bare_soil leaves it without meadow patches.
        // (or, the bare class only, warm bare ground: `NAIP_WARM_RG`, soil showing through).
        // The flora pass's chert ground (`--no-chert-ground`) shrinks the rule with the
        // vegetation's (eo_landmarks::vegetation::bare_soil_with): steeper faces, more
        // soil, and only the warmest ground (the summer image's cured grass is warm too).
        let chert = eo_landmarks::flora::flora().chert;
        let (face, soil_min, warm_min) = if chert { (CHERT_FACE_SLOPE, 0.45, 0.85) } else { (BARE_FACE_SLOPE, 0.3, 0.5) };
        let eroded = eo_landmarks::vegetation::grass3().bare
            && pr == Prior::Bare
            && (slope > face || soil_share(naip, &self.frame, x, z, fp, eo_landmarks::vegetation::NAIP_SOIL_RG) > soil_min || soil_share(naip, &self.frame, x, z, fp, eo_landmarks::vegetation::NAIP_WARM_RG) > warm_min);
        if pr == Prior::Bare && tone != Some("bareRock") && eo_landmarks::vegetation::grass2().cover && !eroded {
            pr = Prior::Grass;
        }
        let (grass, canopy) = cover_shares(pr, veg, dark_veg, pale, slope);
        let lin = |c: [u8; 3]| c.map(|v| srgb_lin(v));
        // the canopy: drawn as the coastal scrub's own albedo (the far layer's, farshore.rs
        // FAR_SCRUB) between the planted shrubs
        let scrub = crate::farshore::FAR_SCRUB.map(|v| v as f64);
        let substrate = if lat > MARIN_LAT {
            // (the flora pass: the red chert soil, the knoll's measured top and cut face.)
            // Above the waterline cliffs only: their base is basalt and greenstone, dark
            // (guide 4.1; the shaded Lime Point cliffs measure (71,67,64)).
            let (top, cut) = if chert && y as f64 > eo_landmarks::flora::CHERT_MIN_Y { (eo_landmarks::flora::CHERT_TOP_SRGB, eo_landmarks::flora::CHERT_FACE_SRGB) } else { (SOIL_SURFACE_SRGB, SOIL_ERODED_SRGB) };
            let soil = mix3(lin(top), lin(cut), smooth(ERODED_SLOPE[0], ERODED_SLOPE[1], slope));
            mix3(soil, scrub, canopy)
        } else {
            let own = [0, 8, 16].map(|s| srgb_lin(((rgba >> s) & 255) as u8));
            mix3(own, scrub, canopy)
        };
        let c = enc(substrate[0]) | (enc(substrate[1]) << 8) | (enc(substrate[2]) << 16) | (MATERIAL_ALPHA << 24);
        (c, (grass.clamp(0.0, 1.0) * 127.0).round() as i8)
    }
}

/// The second flora pass's knoll at game (x, z) (eo_landmarks::flora2::knoll):
/// (top, face, the face's downslope direction) from the 1 m DEM's convexity
/// (its height over the means of rings 12 and 25 m out, as the plants') and
/// slope, and the NAIP's green share over 2.5 m; None off Marin, the DEM or
/// the image, or where the ground is not convex.
pub fn knoll_at(inp: &Inputs, dem: &Dem1m, x: f64, z: f64) -> Option<(f64, f64, [f64; 2])> {
    let h = |x: f64, z: f64| -> Option<f64> {
        let (lat, lon) = inp.frame.lat_lon(x, z);
        dem.height(lon, lat).map(|v| v as f64 * inp.height_scale)
    };
    let (lat, _) = inp.frame.lat_lon(x, z);
    if lat <= MARIN_LAT {
        return None;
    }
    let c = h(x, z)?;
    let r = eo_landmarks::flora2::KNOLL_RING_M.0;
    let mut ring = 0.0;
    for k in 0..8 {
        let (sn, cs) = (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos();
        ring += h(x + cs * r, z + sn * r)?;
    }
    let hollow = ring / 8.0 - c;
    if -hollow < eo_landmarks::flora2::KNOLL_CONVEX_M.0 {
        return None;
    }
    let r2 = eo_landmarks::flora2::KNOLL_RING_M.1;
    let mut ring2 = 0.0;
    for k in 0..8 {
        let (sn, cs) = (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos();
        ring2 += h(x + cs * r2, z + sn * r2)?;
    }
    let hollow25 = ring2 / 8.0 - c;
    let e = 1.5;
    let gx = (h(x + e, z)? - h(x - e, z)?) / (2.0 * e);
    let gz = (h(x, z + e)? - h(x, z - e)?) / (2.0 * e);
    let g = gx.hypot(gz);
    let slope = g.atan().to_degrees();
    let naip = naip()?;
    let (mut green, mut n) = (0.0, 0.0);
    for j in 0..5 {
        for i in 0..5 {
            let (la, lo) = inp.frame.lat_lon(x + (i as f64 - 2.0) * 0.5, z + (j as f64 - 2.0) * 0.5);
            let Some(p) = naip.sample_rgb(lo, la) else { continue };
            let [r, g, b] = p.map(|c| c as f64);
            n += 1.0;
            if 2.0 * g - r - b > eo_landmarks::vegetation::NAIP_GREEN_STRICT as f64 * (r + g + b) && g > r {
                green += 1.0;
            }
        }
    }
    if n == 0.0 {
        return None;
    }
    let (a, b) = eo_landmarks::flora2::KNOLL_CONVEX25_DEM_M;
    let c25 = smooth(a, b, -hollow25);
    let (t, f) = eo_landmarks::flora2::knoll(slope, hollow, c25, green / n);
    let down = if g > 1e-6 { [-gx / g, -gz / g] } else { [1.0, 0.0] };
    Some((t, f, down))
}

/// A material vertex on the knoll (`kn`: its (top, face)): the top's rust soil
/// and the face's eroded chert (eo_landmarks::flora2 `KNOLL_TOP_SRGB`,
/// `KNOLL_FACE_SRGB`), and the shader's knoll code in the grass byte: -1 to -63
/// (cc0.wgsl `gg3_terrain`: sparse cured grass, rubble, rills).
pub fn knoll_material(c: u32, w: i8, kn: (f64, f64)) -> (u32, i8) {
    let k = (kn.0 + kn.1).min(1.0);
    if k <= 0.02 || (c >> 24) != MATERIAL_ALPHA {
        return (c, w);
    }
    let lin = |v: [u8; 3]| v.map(srgb_lin);
    let soil = mix3(lin(eo_landmarks::flora2::KNOLL_TOP_SRGB), lin(eo_landmarks::flora2::KNOLL_FACE_SRGB), kn.1 / (kn.0 + kn.1));
    let own = [0, 8, 16].map(|s| srgb_lin(((c >> s) & 255) as u8));
    let m = mix3(own, soil, k);
    let enc = |v: f64| (v.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u32;
    let c = enc(m[0]) | (enc(m[1]) << 8) | (enc(m[2]) << 16) | (MATERIAL_ALPHA << 24);
    // the knoll's code (the shader draws the rills where the face is steep)
    (c, -(1 + (62.0 * k).round() as i8))
}

/// The NAIP over a square of side `fp` at (x, z), 5 x 5 taps: (vegetation
/// share, its dark (canopy) part, pale ground share); None off the imagery.
pub fn naip_shares(naip: &Naip, frame: &GeoFrame, x: f64, z: f64, fp: f64) -> Option<(f64, f64, f64)> {
    let (mut veg, mut dark_veg, mut pale, mut n) = (0.0, 0.0, 0.0, 0.0);
    for j in 0..5 {
        for i in 0..5 {
            let (px, pz) = (x + (i as f64 - 2.0) * fp / 5.0, z + (j as f64 - 2.0) * fp / 5.0);
            let (lat, lon) = frame.lat_lon(px, pz);
            let Some(c) = naip.sample_rgb(lon, lat) else { continue };
            let (r, g, b) = (c[0] as f64, c[1] as f64, c[2] as f64);
            let s = (r + g + b).max(1.0);
            let v = smooth(EXG_BARE, EXG_FULL, (2.0 * g - r - b) / s);
            let yel = ((r + g) * 0.5 - b) / (s / 3.0);
            veg += v;
            dark_veg += v * smooth(CANOPY_BRIGHT[1], CANOPY_BRIGHT[0], s / 3.0);
            pale += smooth(PALE_BRIGHT[0], PALE_BRIGHT[1], s / 3.0) * smooth(PALE_YELLOW[0], PALE_YELLOW[1], yel) * (1.0 - v);
            n += 1.0;
        }
    }
    (n > 0.0).then(|| (veg / n, dark_veg / n, pale / n))
}

/// The slope (rise over run) past which the bare class stays eroded soil under
/// the third grass pass (`--no-bare-ground`): 0.84, 40 degrees, the middle of
/// eo_landmarks::vegetation::bare_soil's 36-46 degree ramp.
pub const BARE_FACE_SLOPE: f64 = 0.84;
/// The same under the flora pass's chert ground: 1.0, 45 degrees, the middle of
/// eo_landmarks::vegetation::bare_soil_with's shrunk 40-50 degree ramp.
pub const CHERT_FACE_SLOPE: f64 = 1.0;

/// The share of red-brown soil in the NAIP over a square of side `fp` at (x, z)
/// (5 x 5 taps): not green, red over green by `rg` and not pale
/// (eo_landmarks::vegetation::Naip::soil's and `warm`'s rule).
pub fn soil_share(naip: &Naip, frame: &GeoFrame, x: f64, z: f64, fp: f64, rg: f32) -> f64 {
    let (mut acc, mut n) = (0.0, 0.0);
    for j in 0..5 {
        for i in 0..5 {
            let (lat, lon) = frame.lat_lon(x + (i as f64 - 2.0) * fp / 5.0, z + (j as f64 - 2.0) * fp / 5.0);
            let Some(c) = naip.sample_rgb(lon, lat) else { continue };
            let (r, g, b) = (c[0] as f64, c[1] as f64, c[2] as f64);
            let green = 2.0 * g - r - b > 0.0 && g > r;
            let rg = rg as f64;
            if !green && r >= rg * g && r > b && (r + g + b) / 765.0 <= eo_landmarks::vegetation::NAIP_BARE_V as f64 {
                acc += 1.0;
            }
            n += 1.0;
        }
    }
    if n > 0.0 { acc / n } else { 0.0 }
}

/// An sRGB byte as linear.
pub fn srgb_lin(v: u8) -> f64 {
    eo_data::roof_colours::srgb_to_linear(v as f32 / 255.0) as f64
}

/// The share of pale bare ground (gravel, paths, concrete slabs: `PALE_*`)
/// in the NAIP over a square of side `fp` at (x, z) (3 x 3 taps; 0 without the imagery).
pub fn pale_ground(frame: &GeoFrame, x: f64, z: f64, fp: f64) -> f64 {
    let Some(naip) = naip() else { return 0.0 };
    let (mut acc, mut n) = (0.0, 0.0);
    for j in 0..3 {
        for i in 0..3 {
            let (lat, lon) = frame.lat_lon(x + (i as f64 - 1.0) * fp / 3.0, z + (j as f64 - 1.0) * fp / 3.0);
            let Some(c) = naip.sample_rgb(lon, lat) else { continue };
            let (r, g, b) = (c[0] as f64, c[1] as f64, c[2] as f64);
            let s = (r + g + b).max(1.0);
            let v = smooth(EXG_BARE, EXG_FULL, (2.0 * g - r - b) / s);
            let yel = ((r + g) * 0.5 - b) / (s / 3.0);
            acc += smooth(PALE_BRIGHT[0], PALE_BRIGHT[1], s / 3.0) * smooth(PALE_YELLOW[0], PALE_YELLOW[1], yel) * (1.0 - v);
            n += 1.0;
        }
    }
    if n > 0.0 { acc / n } else { 0.0 }
}

fn mix3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cell_split_matches_the_tiles() {
        // corners and the diagonal from (1, 0) to (0, 1)
        assert_eq!(cell_tri(1.0, 2.0, 3.0, 4.0, 0.0, 0.0), 1.0);
        assert_eq!(cell_tri(1.0, 2.0, 3.0, 4.0, 1.0, 0.0), 2.0);
        assert_eq!(cell_tri(1.0, 2.0, 3.0, 4.0, 0.0, 1.0), 3.0);
        assert_eq!(cell_tri(1.0, 2.0, 3.0, 4.0, 1.0, 1.0), 4.0);
        // on the diagonal both triangles agree
        for t in [0.25f32, 0.5, 0.75] {
            let a = 1.0 + t * 1.0 + (1.0 - t) * 2.0;
            assert!((cell_tri(1.0, 2.0, 3.0, 4.0, t, 1.0 - t) - a).abs() < 1e-6);
        }
        // a patch is the tile topology (33 x 33) and whole coarse cells
        assert_eq!(SIDE, crate::scene::TILE_CELLS + 1);
        assert_eq!(PATCH_CELLS % crate::scene::TERRAIN_COARSE_STRIDE as i64, 0);
    }

    #[test]
    fn the_patches_cover_each_eye() {
        let l = Lattice { ox: -12000.0, oz: -9000.0, cell: 8.0, width: 3000, height: 3000 };
        let keys: HashSet<_> = patch_keys(&l).into_iter().collect();
        for (_, c) in REGIONS {
            for (dx, dz) in [(0.0, 0.0), (390.0, 0.0), (-390.0, 0.0), (0.0, 390.0), (0.0, -390.0)] {
                let cx = ((c[0] + dx - l.ox) / l.cell - 0.5).floor() as i64;
                let cz = ((c[1] + dz - l.oz) / l.cell - 0.5).floor() as i64;
                assert!(keys.contains(&Lattice::patch_of_quad(cx, cz)), "{c:?} + ({dx}, {dz})");
            }
        }
    }

    #[test]
    fn the_soil_tones_have_their_munsell_values() {
        let y = |c: [u8; 3]| {
            let l = c.map(srgb_lin);
            0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
        };
        // Munsell value V has luminance about (V / 10)^2 x 1.21 ... the renotation: 3 -> 0.066, 4 -> 0.12, 5 -> 0.198
        assert!((y(SOIL_ERODED_SRGB) - 0.120).abs() < 0.01, "{}", y(SOIL_ERODED_SRGB));
        assert!((y(SOIL_SURFACE_SRGB) - 0.198).abs() < 0.01, "{}", y(SOIL_SURFACE_SRGB));
        // 5YR: red over green over blue
        for c in [SOIL_ERODED_SRGB, SOIL_SURFACE_SRGB] {
            assert!(c[0] > c[1] && c[1] > c[2]);
        }
    }

    #[test]
    fn the_cover_shares_follow_the_prior() {
        // grassland: grass but for shrubs and pale ground; a face past the sward's slope is soil
        assert_eq!(cover_shares(Prior::Grass, 0.1, 0.0, 0.0, 0.2), (1.0, 0.0));
        let (g, c) = cover_shares(Prior::Grass, 0.6, 0.4, 0.1, 0.2);
        assert!((g - 0.5).abs() < 1e-9 && (c - 0.4).abs() < 1e-9);
        assert_eq!(cover_shares(Prior::Grass, 0.1, 0.0, 0.0, SWARD_SLOPE[1]).0, 0.0);
        // scrub: the canopy is the vegetation, the openings grass
        let (g, c) = cover_shares(Prior::Scrub, 0.7, 0.2, 0.0, 0.3);
        assert!((g - 0.3).abs() < 1e-9 && (c - 0.7).abs() < 1e-9);
        // bare: soil, but for the green herbs
        let (g, c) = cover_shares(Prior::Bare, 0.3, 0.1, 0.0, 0.3);
        assert!((g - 0.2).abs() < 1e-9 && (c - 0.1).abs() < 1e-9);
        assert_eq!(prior(None, 30), Prior::Grass);
        assert_eq!(prior(None, 10), Prior::Scrub);
        assert_eq!(prior(Some("bareEarth"), 30), Prior::Bare);
    }

    /// With the survey: the cover shares over the whole Marin extent of the
    /// NAIP, away from the headland eye's square, by WorldCover class
    /// (printed: the method's histogram); WorldCover grassland reads as grass.
    #[test]
    fn the_marin_cover_follows_the_land_cover() {
        let (Some(naip), Some(dem)) = (Root::discover().and_then(|_| naip()), dem()) else {
            eprintln!("the_marin_cover_follows_the_land_cover: skipped, no survey root");
            return;
        };
        let Ok(paths) = eo_data::DataPaths::discover() else { return };
        let Ok(land) = eo_data::LandCover::load(&paths) else { return };
        let frame = GeoFrame::new(eo_data::roof_colours::PlanFrame { center_lat: 37.778, center_lon: -122.4194, m_per_deg_lat: 110574.0, m_per_deg_lon: 87986.24747640654 }, 1.25, 1.25);
        let game = |lon: f64, lat: f64| ((lon + 122.4194) * 87986.2475 * 1.25, (37.778 - lat) * 110574.0 * 1.25);
        let eye = REGIONS[0].1;
        let mut by: HashMap<u8, Vec<(f64, f64, f64, f64)>> = HashMap::new();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut tries = 0;
        while by.values().map(|v| v.len()).sum::<usize>() < 6000 && tries < 200_000 {
            tries += 1;
            let (lon, lat) = (-122.5064 + rnd() * 0.0556, MARIN_LAT + rnd() * 0.0209);
            let (x, z) = game(lon, lat);
            if (x - eye[0]).abs() < REGION_HALF_M && (z - eye[1]).abs() < REGION_HALF_M {
                continue;
            }
            let class = land.class_at(x, z);
            if !matches!(class, 10 | 20 | 30 | 60) {
                continue;
            }
            let h = |dx: f64, dz: f64| {
                let (la, lo) = frame.lat_lon(x + dx, z + dz);
                dem.height(lo, la).map(|v| v as f64 * 1.25)
            };
            let (Some(a), Some(b), Some(c), Some(d)) = (h(-4.0, 0.0), h(4.0, 0.0), h(0.0, -4.0), h(0.0, 4.0)) else { continue };
            let slope = ((b - a) / 8.0).hypot((d - c) / 8.0);
            let Some((veg, dark, pale)) = naip_shares(naip, &frame, x, z, 2.0) else { continue };
            let (g, c) = cover_shares(prior(None, class), veg, dark, pale, slope);
            by.entry(class).or_default().push((g, c, pale, veg));
        }
        let mut grass_median = 0.0;
        for class in [10u8, 20, 30, 60] {
            let Some(v) = by.get(&class) else { continue };
            let mut hist = [0usize; 5];
            let mut gs: Vec<f64> = v.iter().map(|r| r.0).collect();
            for &g in &gs {
                hist[((g * 5.0) as usize).min(4)] += 1;
            }
            gs.sort_by(f64::total_cmp);
            let mut cs: Vec<f64> = v.iter().map(|r| r.1).collect();
            cs.sort_by(f64::total_cmp);
            let med = gs[gs.len() / 2];
            eprintln!(
                "WorldCover {class}: n {} grass share 0-.2 {} | .2-.4 {} | .4-.6 {} | .6-.8 {} | .8-1 {}; median grass {:.2} canopy {:.2}; pale over 0.5 {}",
                v.len(),
                hist[0],
                hist[1],
                hist[2],
                hist[3],
                hist[4],
                med,
                cs[cs.len() / 2],
                v.iter().filter(|r| r.2 > 0.5).count()
            );
            if class == 30 {
                grass_median = med;
            }
        }
        assert!(grass_median >= 0.5, "WorldCover grassland reads as grass: median share {grass_median}");
    }

    /// With the survey: on a flat 8 m ground every patch's outer edge lies on
    /// the 8 m surface (no crack), the headland gets relief at the drawn
    /// height scale, and the heights the props read are the drawn ones.
    #[test]
    fn the_relief_meets_the_tiles_and_follows_the_dem() {
        if Root::discover().is_none() || dem().is_none() {
            eprintln!("the_relief_meets_the_tiles_and_follows_the_dem: skipped, no survey root");
            return;
        }
        let frame = GeoFrame::new(eo_data::roof_colours::PlanFrame { center_lat: 37.778, center_lon: -122.4194, m_per_deg_lat: 110574.0, m_per_deg_lon: 87986.24747640654 }, 1.25, 1.25);
        let l = Lattice { ox: -9000.0, oz: -8000.0, cell: 8.0, width: 1200, height: 1200 };
        let flat = |_: i64, _: i64| 100.0f32;
        let raw = |_: f64, _: f64| 100.0f32;
        let colour = |_: f32, _: f32| 0xff80_8080u32;
        let inp = Inputs { lattice: l, vertex_height: &flat, raw_ground: &raw, colour: &colour, street_tris: &[], buildings: vec![], frame, height_scale: 1.25, sea_y: 0.0 };
        let d = build(&inp, None).expect("patches");
        assert!(d.stats.relieved > 1000, "{:?}", d.stats);
        // the patches over the sea (no DEM there) carry no relief: dropped, their neighbours rebuilt
        assert!(d.stats.dropped > 0 && d.stats.rebuilt > 0, "{:?}", d.stats);
        assert_eq!(d.keys.len(), d.patches.len());
        let up = eo_assets::glb::pack_normal(glam::Vec3::Y) & 0x00ff_ffff;
        for p in &d.patches {
            assert!(p.relieved > 0 || p.material > 0, "a flat patch is dropped: {:?}", p.key);
            let has = |dx: i64, dz: i64| d.keys.contains(&(p.key.0 + dx, p.key.1 + dz));
            for (k, v) in p.verts.iter().enumerate() {
                let (i, j) = (k % SIDE, k / SIDE);
                let (w, e, n, s) = (i == 0, i == SIDE - 1, j == 0, j == SIDE - 1);
                // an edge or a corner with no patch beyond lies on the 8 m surface (no crack)
                let outer = (w && !has(-1, 0))
                    || (e && !has(1, 0))
                    || (n && !has(0, -1))
                    || (s && !has(0, 1))
                    || (w && n && !has(-1, -1))
                    || (e && n && !has(1, -1))
                    || (w && s && !has(-1, 1))
                    || (e && s && !has(1, 1));
                if outer {
                    assert!((v.pos[1] - 100.0).abs() < 1e-4, "patch {:?} edge vertex {k}: {}", p.key, v.pos[1]);
                }
                assert!((v.pos[1] - 100.0).abs() <= MAX_RELIEF_M + 1e-4);
                assert!((v.pos[1] - 100.0 - p.relief[k]).abs() < 1e-4);
                // the provider: the drawn height where the relief moved the ground, None where
                // the patch draws the 8 m surface (the readers' own ground is the drawn one there)
                if !e && !s {
                    let (x, z) = (v.pos[0] as f64 + 0.3, v.pos[2] as f64 + 0.2);
                    let h = d.height(x, z).expect("inside the patch");
                    match d.relief_height(x, z) {
                        Some(r) => assert!((r - h).abs() < 1e-9),
                        None => assert!((h - 100.0).abs() < RELIEF_EPS_M as f64 + 1e-4, "{h}"),
                    }
                }
                // no relief round a vertex: the tiles' own normal (flat 8 m ground: up)
                if i > 0 && j > 0 && !e && !s && [k - 1, k + 1, k - SIDE, k + SIDE].iter().all(|&q| p.relief[q] == 0.0) {
                    assert_eq!(v.normal & 0x00ff_ffff, up, "patch {:?} vertex {k}", p.key);
                }
            }
        }
        // the props read the drawn surface: a patch vertex's own height
        let p = &d.patches[d.patches.len() / 2];
        let v = p.verts[SIDE * 10 + 12];
        let h = d.height(v.pos[0] as f64, v.pos[2] as f64).unwrap();
        assert!((h - v.pos[1] as f64).abs() < 1e-3, "{h} {}", v.pos[1]);
        // no relief within EYE_CLEAR_M of an eye
        for (_, e) in REGIONS {
            for (dx, dz) in [(0.0, 0.0), (10.0, 0.0), (0.0, -10.0), (-7.0, 7.0)] {
                if let Some(h) = d.height(e[0] + dx, e[1] + dz) {
                    assert!((h - 100.0).abs() < 1e-4, "eye {e:?} + ({dx}, {dz}): {h}");
                }
            }
        }
    }
}
