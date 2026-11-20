//! The ambient volume's bake (`LightingOptions::ambient_volume`; the GPU side
//! is ambient.rs): large-scale sky visibility from the building prisms, the
//! terrain and, with `--structures`, the decks, per cell of a
//! terrain-following grid, as four L1 coefficients of the visible sky's
//! irradiance. Pure CPU and deterministic: a block is a function of its
//! position and the scene only (never of the thread that bakes it or of the
//! order), so tiles baked apart meet without a seam.
//!
//! A cell stores `(a0, ax, ay, az)`: the visible sky's share of a whole
//! hemisphere's irradiance on a surface with normal `n` is
//! `a0 + ax n.x + ay n.y + az n.z` (`E_visible(n) / pi` for a sky of unit
//! radiance), the form of the L1 band of spherical harmonics, fitted to the
//! exact irradiance facing up and on the horizontal (`coefficients`) rather
//! than projected. `OPEN` = `(0.5, 0, 0.5, 0)` is the open upper hemisphere,
//! for which the shading is the frame without the volume. Only the upper
//! hemisphere counts as sky (below it is the ground, the rig's ground lobe),
//! so an open plaza bakes to `OPEN` exactly.
//!
//! The bake is a horizon scan, as the far shore's sky-visibility bake
//! (eo-farshore `bake_sky_visibility`): roofs and the terrain are rasterised
//! into a height field round the block with a margin of the reach
//! (`REACH_M`), and from each cell `Spec::azimuths` sectors are marched out
//! to the reach, keeping the highest horizon. Decks are overhangs, not
//! columns: they are kept in a second layer (bottom and top), so a cell under
//! a deck loses the sky above it and keeps the open sides. The sectors turn
//! by a quarter sector from one cell to the next (a fixed pattern of the
//! cell's global index), and a 3 x 3 tent over the cells in the open takes
//! out the pattern. Cells inside a building or a deck are filled from their
//! neighbours in the open (`DILATE_RINGS` rings, then from above), so a
//! wall's or a roof's filtered sample never reads the inside.
//!
//! With `Spec::ground` (the volume levels while the ground bounce is on,
//! `LightingOptions::ground_bounce`) each cell also stores the open ground's
//! share of its lower hemisphere, `(g0, gx, gy, gz)` in the same form
//! (`g0 + g . n`, fitted exact facing down and on the horizontal): the
//! ground that is the top surface of its column (the street, a roof, a deck's
//! top: nothing over it, so sunlit while the sun is up), against what the
//! lower hemisphere sees otherwise (the ground under a deck, a lower deck
//! under a higher one, walls below the horizon). The same march finds it: in
//! each sector the floor below the cell is swept outward, the highest
//! depression seen so far hiding what lies behind it, and the bands of
//! depression angle that land on open ground are integrated. Open flat
//! ground bakes to `OPEN_GROUND` exactly; beyond the reach counts as open.
//! The share is then weighted by how likely the ground round the cell is to
//! be in the sun (`SunShare`: its horizon of buildings and terrain), so a
//! narrow alley's shaded floor bounces little; a deck's own shadow is the
//! shader's (it reads undersides shifted toward the sun). The shader turns it
//! into light bounced up from the sunlit ground (ambient.wgsl `amb_bounce`).

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::Arc;

/// Bumped when a baked block would change for the same inputs (2: the open
/// ground's share beside the sky's, `Spec::ground`).
pub const BAKE_VERSION: u32 = 2;
/// The bake's own source: its digest is part of the cache key, so a change
/// to the bake code bakes into a new folder.
pub const BAKE_SOURCE: &str = include_str!("ambient_bake.rs");
pub const CACHE_MAGIC: &[u8; 4] = b"EOAV";

/// Side of a volume tile (the street and kit tiles' 128 m), metres.
pub const TILE_M: f64 = 128.0;
/// How far the horizon scan looks, metres: past it the sky counts as open.
/// A 40 m wall 160 m away still stands 14 deg over the horizon; the citywide
/// plane's coarse cells carry what is further.
pub const REACH_M: f64 = 160.0;
/// Sectors marched per volume cell (and per plane cell).
pub const AZIMUTHS: usize = 32;
pub const PLANE_AZIMUTHS: usize = 16;
/// The volume's layers above the ground (`layer_centre`): `FINE_LAYERS` of
/// `FINE_LAYER_M` up to 32 m (the street, the lower floors, the canyon's
/// depth over most of the city), then `COARSE_LAYERS` of `COARSE_LAYER_M` up
/// to `TOP_M` (the downtown towers' middle floors). Above, the shading eases
/// the top layer to the open sky by the plane's reach.
pub const FINE_LAYERS: usize = 16;
pub const FINE_LAYER_M: f64 = 2.0;
pub const COARSE_LAYERS: usize = 8;
pub const COARSE_LAYER_M: f64 = 6.0;
pub const LAYERS: usize = FINE_LAYERS + COARSE_LAYERS;
pub const TOP_M: f64 = FINE_LAYERS as f64 * FINE_LAYER_M + COARSE_LAYERS as f64 * COARSE_LAYER_M;
/// The citywide plane's cell (street-level visibility beyond the volume), metres.
pub const PLANE_CELL_M: f64 = 8.0;
/// The plane is baked in blocks of this many cells a side (1,024 m).
pub const PLANE_TILE_CELLS: usize = 128;
/// The plane's extent is rounded out to this (a whole 256-byte texture row: 32 cells).
pub const PLANE_ALIGN_M: f64 = 256.0;
/// Heights above the ground at which the plane's bake looks (the first is
/// the street level the plane stores; the others find its reach).
pub const PLANE_HEIGHTS: [f64; 7] = [1.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
/// The plane's reach: the height above the ground at which an upward
/// surface sees this share of the open sky.
pub const PLANE_OPEN_SHARE: f64 = 0.95;
/// Cells baked round a block on every side and cropped (the tent's one cell
/// and the fill's rings), so a block's cells are those of a bake of the
/// whole city.
pub const APRON: usize = 4;
pub const DILATE_RINGS: usize = 3;
/// A cell's centre stands at least this far over the true ground (the
/// terrain-following grid follows the plane's coarser ground).
pub const MIN_ABOVE_GROUND_M: f64 = 0.25;
/// The thinnest deck slab an occluder is given (a landmark's deck has no
/// superstructure depth in the profile), metres. INFERRED: a slab's own
/// thickness, about a metre (reference sheet 1.3: 8 in deck overhangs over
/// 4-6 ft girders); the decks' own depths from the profile override it.
pub const MIN_DECK_SLAB_M: f64 = 1.0;
/// Deck stations are sampled this far apart along a chain (metres).
pub const DECK_STEP_M: f64 = 2.0;
/// The march's step grows with the distance: one raster cell near, then this share of the distance.
pub const STEP_GROWTH: f64 = 0.06;
/// The open upper hemisphere (`(a0, ax, ay, az)`).
pub const OPEN: [f32; 4] = [0.5, 0.0, 0.5, 0.0];
/// A lower hemisphere of open ground (`(g0, gx, gy, gz)`): the whole of it
/// facing down, half of it facing sideways, none facing up.
pub const OPEN_GROUND: [f32; 4] = [0.5, 0.0, -0.5, 0.0];
/// Bucket of the occluder index, metres.
const BUCKET_M: f64 = 64.0;
/// Most layers a spec may have.
pub const MAX_LAYERS: usize = 32;

/// Height above the ground of volume layer `k`'s centre.
pub fn layer_centre(k: usize) -> f64 {
    if k < FINE_LAYERS {
        (k as f64 + 0.5) * FINE_LAYER_M
    } else {
        FINE_LAYERS as f64 * FINE_LAYER_M + ((k - FINE_LAYERS) as f64 + 0.5) * COARSE_LAYER_M
    }
}

/// The ground under the scene (the true one: occluders and cell floors).
pub trait Ground: Send + Sync {
    fn ground(&self, x: f64, z: f64) -> f64;
}

/// A flat ground (tests).
pub struct FlatGround(pub f64);

impl Ground for FlatGround {
    fn ground(&self, _x: f64, _z: f64) -> f64 {
        self.0
    }
}

/// The city's elevation grid, never under the sea's surface (the volume
/// follows the water, not the bay floor).
pub struct ElevationGround(pub Arc<eo_data::ElevationGrid>);

impl Ground for ElevationGround {
    fn ground(&self, x: f64, z: f64) -> f64 {
        self.0.ground(x, z).max(eo_data::SEA_LEVEL_Y)
    }
}

/// A building as an occluder: its footprint and the height of its roof.
#[derive(Clone, Debug)]
pub struct Prism {
    pub ring: Vec<[f64; 2]>,
    pub top: f64,
}

impl Prism {
    /// A footprint standing on the ground as the collision prisms stand
    /// (eo-player: on the highest ground under its ring, lifted by the
    /// construction lift, `height` tall).
    pub fn standing(ring: Vec<[f64; 2]>, height: f64, lift: f64, ground: &dyn Ground) -> Option<Self> {
        if ring.len() < 3 || !(height > 0.0) {
            return None;
        }
        let hi = ring.iter().map(|p| ground.ground(p[0], p[1])).fold(f64::MIN, f64::max);
        Some(Self { ring, top: hi + lift + height })
    }
}

/// A deck between two stations: centreline ends, half widths, top and bottom at each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckPiece {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub half: [f64; 2],
    pub top: [f64; 2],
    pub bottom: [f64; 2],
}

/// The decks of the built chains (deck and landmark kinds: a tunnel or a
/// road below others is not an overhang), sampled every `DECK_STEP_M`. Uses
/// only `chains`, `is_built`, `kind`, `length`, `frame`, `y_at`, `half_at`
/// and `depth_at`.
pub fn deck_pieces(world: &eo_structures::StructureWorld) -> Vec<DeckPiece> {
    use eo_structures::ChainKind;
    let mut out = vec![];
    for c in world.chains() {
        if !c.is_built() || !matches!(c.kind, ChainKind::Deck | ChainKind::Landmark) {
            continue;
        }
        let len = c.length();
        if !(len > 0.0) {
            continue;
        }
        let n = (len / DECK_STEP_M).ceil().max(1.0) as usize;
        let at = |s: f64| {
            let f = c.frame(s);
            let top = c.y_at(s);
            ([f.x, f.z], c.half_at(s), top, top - c.depth_at(s).max(MIN_DECK_SLAB_M))
        };
        let mut prev = at(0.0);
        for i in 1..=n {
            let s = len * i as f64 / n as f64;
            let cur = at(s);
            out.push(DeckPiece { a: prev.0, b: cur.0, half: [prev.1, cur.1], top: [prev.2, cur.2], bottom: [prev.3, cur.3] });
            prev = cur;
        }
    }
    out
}

/// Where the citywide plane lies: its corner and its cells (`PLANE_CELL_M`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneMap {
    pub min: [f64; 2],
    pub w: usize,
    pub h: usize,
}

impl PlaneMap {
    /// Round the occluders' bounds (min x, min z, max x, max z) with the reach
    /// on every side, out to `PLANE_ALIGN_M`.
    pub fn around(bounds: [f64; 4]) -> Self {
        let a = PLANE_ALIGN_M;
        let x0 = ((bounds[0] - REACH_M) / a).floor() * a;
        let z0 = ((bounds[1] - REACH_M) / a).floor() * a;
        let x1 = ((bounds[2] + REACH_M) / a).ceil() * a;
        let z1 = ((bounds[3] + REACH_M) / a).ceil() * a;
        Self { min: [x0, z0], w: ((x1 - x0) / PLANE_CELL_M).round().max(32.0) as usize, h: ((z1 - z0) / PLANE_CELL_M).round().max(32.0) as usize }
    }

    /// A plane of `texels` a side centred on the occluders' bounds (min x,
    /// min z, max x, max z), its corner on `PLANE_ALIGN_M`.
    pub fn centred(bounds: [f64; 4], texels: usize) -> Self {
        let a = PLANE_ALIGN_M;
        let half = 0.5 * texels as f64 * PLANE_CELL_M;
        let c = [0.5 * (bounds[0] + bounds[2]), 0.5 * (bounds[1] + bounds[3])];
        Self { min: [((c[0] - half) / a).round() * a, ((c[1] - half) / a).round() * a], w: texels, h: texels }
    }

    /// Whether the plane holds the bounds and the reach round them.
    pub fn holds(&self, b: [f64; 4]) -> bool {
        let m = self.max();
        b[0] - REACH_M >= self.min[0] && b[1] - REACH_M >= self.min[1] && b[2] + REACH_M <= m[0] && b[3] + REACH_M <= m[1]
    }

    pub fn max(&self) -> [f64; 2] {
        [self.min[0] + self.w as f64 * PLANE_CELL_M, self.min[1] + self.h as f64 * PLANE_CELL_M]
    }

    /// The plane's bake blocks (`PLANE_TILE_CELLS` a side), by index.
    pub fn blocks(&self) -> Vec<(i32, i32)> {
        let (bw, bh) = (self.w.div_ceil(PLANE_TILE_CELLS), self.h.div_ceil(PLANE_TILE_CELLS));
        (0..bh as i32).flat_map(|j| (0..bw as i32).map(move |i| (i, j))).collect()
    }

    /// A block's corner (world metres).
    pub fn block_origin(&self, b: (i32, i32)) -> [f64; 2] {
        let s = PLANE_TILE_CELLS as f64 * PLANE_CELL_M;
        [self.min[0] + b.0 as f64 * s, self.min[1] + b.1 as f64 * s]
    }

    /// Whether a box (min x, min z, max x, max z) lies wholly outside the plane.
    pub fn misses(&self, b: [f64; 4]) -> bool {
        let m = self.max();
        b[2] <= self.min[0] || b[3] <= self.min[1] || b[0] >= m[0] || b[1] >= m[1]
    }
}

/// The ground the shading's terrain-following grid stands on: the plane's
/// ground channel (the true ground at its cell centres, rounded to half
/// floats as the texture holds it), bilinear and clamped at the plane's
/// edge as the GPU samples it. The bake places every cell on it, so the
/// shader's `y - ground` lands on the layer the cell was baked for.
#[derive(Clone, Debug)]
pub struct GroundRef {
    pub map: PlaneMap,
    pub h: Vec<f32>,
}

impl GroundRef {
    /// Sample `ground` at every plane cell's centre (`threads` workers, the same result for any count).
    pub fn sample(map: PlaneMap, ground: &dyn Ground, threads: usize) -> Self {
        let mut h = vec![0f32; map.w * map.h];
        let rows = map.h.div_ceil(threads.max(1)).max(1);
        std::thread::scope(|s| {
            for (k, chunk) in h.chunks_mut(rows * map.w).enumerate() {
                s.spawn(move || {
                    crate::hitch::worker_started();
                    for (i, v) in chunk.iter_mut().enumerate() {
                        let (cx, cz) = (i % map.w, k * rows + i / map.w);
                        let x = map.min[0] + (cx as f64 + 0.5) * PLANE_CELL_M;
                        let z = map.min[1] + (cz as f64 + 0.5) * PLANE_CELL_M;
                        *v = f16_value(f16_bits(ground.ground(x, z) as f32));
                    }
                });
            }
        });
        Self { map, h }
    }

    /// A ground of one height everywhere (tests).
    pub fn flat(map: PlaneMap, y: f64) -> Self {
        Self { map, h: vec![f16_value(f16_bits(y as f32)); map.w * map.h] }
    }

    /// Bilinear between the cell centres, clamped to the outermost ones.
    pub fn at(&self, x: f64, z: f64) -> f64 {
        let m = &self.map;
        let fx = ((x - m.min[0]) / PLANE_CELL_M - 0.5).clamp(0.0, (m.w - 1) as f64);
        let fz = ((z - m.min[1]) / PLANE_CELL_M - 0.5).clamp(0.0, (m.h - 1) as f64);
        let (x0, z0) = (fx.floor() as usize, fz.floor() as usize);
        let (x1, z1) = ((x0 + 1).min(m.w - 1), (z0 + 1).min(m.h - 1));
        let (tx, tz) = (fx - x0 as f64, fz - z0 as f64);
        let v = |i: usize, j: usize| self.h[j * m.w + i] as f64;
        let a = v(x0, z0) + (v(x1, z0) - v(x0, z0)) * tx;
        let b = v(x0, z1) + (v(x1, z1) - v(x0, z1)) * tx;
        a + (b - a) * tz
    }
}

/// Everything a bake reads.
pub struct Scene {
    pub prisms: Vec<Prism>,
    pub decks: Vec<DeckPiece>,
    prism_box: Vec<[f64; 4]>,
    deck_box: Vec<[f64; 4]>,
    prism_cells: HashMap<(i32, i32), Vec<u32>>,
    deck_cells: HashMap<(i32, i32), Vec<u32>>,
    pub ground: Arc<dyn Ground>,
    pub ground_ref: Arc<GroundRef>,
}

fn bucket_range(b: [f64; 4]) -> impl Iterator<Item = (i32, i32)> {
    let (i0, j0) = ((b[0] / BUCKET_M).floor() as i32, (b[1] / BUCKET_M).floor() as i32);
    let (i1, j1) = ((b[2] / BUCKET_M).floor() as i32, (b[3] / BUCKET_M).floor() as i32);
    (j0..=j1).flat_map(move |j| (i0..=i1).map(move |i| (i, j)))
}

fn deck_box(d: &DeckPiece) -> [f64; 4] {
    let h = d.half[0].max(d.half[1]);
    [d.a[0].min(d.b[0]) - h, d.a[1].min(d.b[1]) - h, d.a[0].max(d.b[0]) + h, d.a[1].max(d.b[1]) + h]
}

impl Scene {
    pub fn new(prisms: Vec<Prism>, decks: Vec<DeckPiece>, ground: Arc<dyn Ground>, ground_ref: Arc<GroundRef>) -> Self {
        let prism_box: Vec<[f64; 4]> = prisms
            .iter()
            .map(|p| {
                let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                for q in &p.ring {
                    b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
                }
                b
            })
            .collect();
        let deck_box: Vec<[f64; 4]> = decks.iter().map(deck_box).collect();
        let mut prism_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, b) in prism_box.iter().enumerate() {
            for k in bucket_range(*b) {
                prism_cells.entry(k).or_default().push(i as u32);
            }
        }
        let mut deck_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, b) in deck_box.iter().enumerate() {
            for k in bucket_range(*b) {
                deck_cells.entry(k).or_default().push(i as u32);
            }
        }
        Self { prisms, decks, prism_box, deck_box, prism_cells, deck_cells, ground, ground_ref }
    }

    /// The bounds of every prism and deck (min x, min z, max x, max z), None when there are none.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        self.prism_box.iter().chain(&self.deck_box).fold(None, |acc, b| {
            Some(match acc {
                None => *b,
                Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
            })
        })
    }

    fn query(cells: &HashMap<(i32, i32), Vec<u32>>, boxes: &[[f64; 4]], b: [f64; 4]) -> Vec<u32> {
        let mut ids: Vec<u32> = bucket_range(b).filter_map(|k| cells.get(&k)).flatten().copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|&i| {
            let q = boxes[i as usize];
            q[0] <= b[2] && q[2] >= b[0] && q[1] <= b[3] && q[3] >= b[1]
        });
        ids
    }

    /// A digest of the occluders (the prisms' rings and roofs, the decks).
    pub fn occluder_digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update((self.prisms.len() as u64).to_le_bytes());
        for p in &self.prisms {
            h.update((p.ring.len() as u32).to_le_bytes());
            for q in &p.ring {
                h.update(q[0].to_le_bytes());
                h.update(q[1].to_le_bytes());
            }
            h.update(p.top.to_le_bytes());
        }
        h.update(self.deck_digest());
        h.finalize().into()
    }

    /// A digest of the decks alone (all zero bytes' digest of nothing when there are none).
    pub fn deck_digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update((self.decks.len() as u64).to_le_bytes());
        for d in &self.decks {
            for v in [d.a[0], d.a[1], d.b[0], d.b[1], d.half[0], d.half[1], d.top[0], d.top[1], d.bottom[0], d.bottom[1]] {
                h.update(v.to_le_bytes());
            }
        }
        h.finalize().into()
    }
}

/// What one bake covers: cell size, cells a side, the layers' heights above
/// the ground, sectors, the occluder raster's cell, and whether the open
/// ground's share is baked too (`Block::g`).
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub cell: f64,
    pub cells: usize,
    pub heights: Vec<f64>,
    pub azimuths: usize,
    pub raster: f64,
    pub ground: bool,
}

impl Spec {
    /// A volume level of `cell` metres (2 or 4): 128 m tiles, `LAYERS` layers.
    pub fn volume(cell: f64) -> Self {
        Self { cell, cells: (TILE_M / cell).round() as usize, heights: (0..LAYERS).map(layer_centre).collect(), azimuths: AZIMUTHS, raster: cell * 0.5, ground: false }
    }

    /// The citywide plane: `PLANE_CELL_M` cells in blocks of `PLANE_TILE_CELLS`, the street and the reach heights.
    pub fn plane() -> Self {
        Self { cell: PLANE_CELL_M, cells: PLANE_TILE_CELLS, heights: PLANE_HEIGHTS.to_vec(), azimuths: PLANE_AZIMUTHS, raster: PLANE_CELL_M * 0.5, ground: false }
    }

    /// This spec with the open ground's share baked too (or not).
    pub fn with_ground(self, ground: bool) -> Self {
        Self { ground, ..self }
    }

    /// The bytes the cache key takes from the spec (the ground's tag only when it is baked).
    pub fn key_bytes(&self) -> Vec<u8> {
        let mut v = vec![];
        v.extend_from_slice(&self.cell.to_le_bytes());
        v.extend_from_slice(&(self.cells as u32).to_le_bytes());
        for h in &self.heights {
            v.extend_from_slice(&h.to_le_bytes());
        }
        v.extend_from_slice(&(self.azimuths as u32).to_le_bytes());
        v.extend_from_slice(&self.raster.to_le_bytes());
        if self.ground {
            v.extend_from_slice(b"open-ground");
        }
        v
    }
}

/// A baked block: `cells` x `cells` columns of `layers` cells, `c[(layer * cells + z) * cells + x]`;
/// with the ground baked, `g` the open ground's share in the same order (else empty).
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub cells: usize,
    pub layers: usize,
    pub c: Vec<[f32; 4]>,
    pub g: Vec<[f32; 4]>,
}

impl Block {
    /// The open sky (and, with `ground`, open ground) everywhere.
    pub fn open(cells: usize, layers: usize, ground: bool) -> Self {
        let n = cells * cells * layers;
        Self { cells, layers, c: vec![OPEN; n], g: if ground { vec![OPEN_GROUND; n] } else { vec![] } }
    }

    pub fn at(&self, x: usize, z: usize, layer: usize) -> [f32; 4] {
        self.c[(layer * self.cells + z) * self.cells + x]
    }

    /// The open ground's share at a cell (the ground must have been baked).
    pub fn ground_at(&self, x: usize, z: usize, layer: usize) -> [f32; 4] {
        self.g[(layer * self.cells + z) * self.cells + x]
    }
}

/// The visible share for a direction or normal `n` (`a0 + a . n`, 0..1).
pub fn visibility(c: [f32; 4], n: [f64; 3]) -> f64 {
    (c[0] as f64 + c[1] as f64 * n[0] + c[2] as f64 * n[1] + c[3] as f64 * n[2]).clamp(0.0, 1.0)
}

/// The open ground's share of the lower hemisphere round a normal `n` (0..1,
/// a whole hemisphere's cosine weight; never more than the lower
/// hemisphere's own share, `0.5 - 0.5 n.y`), as the shader takes it.
pub fn ground_share(g: [f32; 4], n: [f64; 3]) -> f64 {
    visibility(g, n).min(0.5 - 0.5 * n[1])
}

/// The occluders round a block: heights (the ground, or a roof over it),
/// the building flag, and up to two deck slabs a cell. Its cells are those
/// of one global lattice (cell `g` centred on `g * r`), so two blocks'
/// rasters agree cell for cell where they overlap and a ray from the same
/// point finds the same cell in both.
struct Raster {
    /// the global index of the first cell.
    g0: [i64; 2],
    r: f64,
    w: usize,
    h: usize,
    height: Vec<f32>,
    building: Vec<bool>,
    /// (bottom, top) of up to two slabs a cell, lowest first; NaN for none. Empty when no deck reaches the raster.
    slabs: Vec<[f32; 4]>,
    /// Nothing but flat ground (no roof, no deck, under half a metre of relief).
    trivial: bool,
}

/// The global raster cell holding `x` (cell `g` covers `[(g - 0.5) r, (g + 0.5) r)`).
#[inline]
fn lattice(x: f64, r: f64) -> i64 {
    (x / r + 0.5).floor() as i64
}

impl Raster {
    fn build(scene: &Scene, r: f64, b: [f64; 4]) -> Self {
        let g0 = [lattice(b[0], r), lattice(b[1], r)];
        let g1 = [lattice(b[2], r), lattice(b[3], r)];
        let w = (g1[0] - g0[0] + 1) as usize;
        let h = (g1[1] - g0[1] + 1) as usize;
        let cx = |i: i64| (g0[0] + i) as f64 * r;
        let cz = |j: i64| (g0[1] + j) as f64 * r;
        let mut height = vec![0f32; w * h];
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for j in 0..h {
            let z = cz(j as i64);
            for i in 0..w {
                let g = scene.ground.ground(cx(i as i64), z) as f32;
                height[j * w + i] = g;
                lo = lo.min(g);
                hi = hi.max(g);
            }
        }
        // the cells whose centres lie in [a, b] along one axis, clamped to the raster.
        let span = |a: f64, bb: f64, g: i64, n: usize| -> Option<(usize, usize)> {
            let i0 = ((a / r).ceil() as i64 - g).max(0);
            let i1 = ((bb / r).floor() as i64 - g).min(n as i64 - 1);
            (i1 >= i0).then_some((i0 as usize, i1 as usize))
        };
        let mut building = vec![false; w * h];
        let ids = Scene::query(&scene.prism_cells, &scene.prism_box, b);
        let mut xs: Vec<f64> = vec![];
        for &id in &ids {
            let p = &scene.prisms[id as usize];
            let pb = scene.prism_box[id as usize];
            let top = p.top as f32;
            let Some((j0, j1)) = span(pb[1], pb[3], g0[1], h) else { continue };
            for j in j0..=j1 {
                let z = cz(j as i64);
                xs.clear();
                let n = p.ring.len();
                for k in 0..n {
                    let (a, c) = (p.ring[k], p.ring[(k + 1) % n]);
                    if (a[1] <= z) != (c[1] <= z) {
                        xs.push(a[0] + (z - a[1]) / (c[1] - a[1]) * (c[0] - a[0]));
                    }
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for pair in xs.chunks_exact(2) {
                    let Some((i0, i1)) = span(pair[0], pair[1], g0[0], w) else { continue };
                    for i in i0..=i1 {
                        let c = j * w + i;
                        if top > height[c] {
                            height[c] = top;
                        }
                        building[c] = true;
                    }
                }
            }
        }
        let decks = Scene::query(&scene.deck_cells, &scene.deck_box, b);
        let mut slabs = vec![];
        if !decks.is_empty() {
            slabs = vec![[f32::NAN; 4]; w * h];
            for &id in &decks {
                let d = scene.decks[id as usize];
                let db = scene.deck_box[id as usize];
                let (Some((i0, i1)), Some((j0, j1))) = (span(db[0], db[2], g0[0], w), span(db[1], db[3], g0[1], h)) else { continue };
                let ab = [d.b[0] - d.a[0], d.b[1] - d.a[1]];
                let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let p = [cx(i as i64), cz(j as i64)];
                        let t = if l2 > 1e-12 { (((p[0] - d.a[0]) * ab[0] + (p[1] - d.a[1]) * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
                        let q = [d.a[0] + ab[0] * t, d.a[1] + ab[1] * t];
                        let lat = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                        if lat > d.half[0] + (d.half[1] - d.half[0]) * t {
                            continue;
                        }
                        let bot = (d.bottom[0] + (d.bottom[1] - d.bottom[0]) * t) as f32;
                        let top = (d.top[0] + (d.top[1] - d.top[0]) * t) as f32;
                        merge_slab(&mut slabs[j * w + i], bot, top);
                    }
                }
            }
        }
        let trivial = ids.is_empty() && slabs.is_empty() && hi - lo < 0.5;
        Self { g0, r, w, h, height, building, slabs, trivial }
    }

    #[inline]
    fn index(&self, x: f64, z: f64) -> Option<usize> {
        let i = lattice(x, self.r) - self.g0[0];
        let j = lattice(z, self.r) - self.g0[1];
        (i >= 0 && j >= 0 && (i as usize) < self.w && (j as usize) < self.h).then(|| j as usize * self.w + i as usize)
    }
}

/// Add a slab (bottom, top) to a cell's two: overlapping slabs merge; a
/// third is merged into the nearer.
fn merge_slab(s: &mut [f32; 4], bot: f32, top: f32) {
    let (bot, top) = (bot.min(top), bot.max(top));
    for k in 0..2 {
        let (b, t) = (s[2 * k], s[2 * k + 1]);
        if b.is_nan() {
            s[2 * k] = bot;
            s[2 * k + 1] = top;
            sort_slabs(s);
            return;
        }
        if bot <= t && top >= b {
            s[2 * k] = b.min(bot);
            s[2 * k + 1] = t.max(top);
            sort_slabs(s);
            return;
        }
    }
    // both taken, neither overlaps: into the nearer.
    let gap = |k: usize| (bot - s[2 * k + 1]).max(s[2 * k] - top);
    let k = if gap(0) <= gap(1) { 0 } else { 1 };
    s[2 * k] = s[2 * k].min(bot);
    s[2 * k + 1] = s[2 * k + 1].max(top);
    sort_slabs(s);
}

fn sort_slabs(s: &mut [f32; 4]) {
    if !s[2].is_nan() && (s[0].is_nan() || s[2] < s[0]) {
        s.swap(0, 2);
        s.swap(1, 3);
    }
    // merge if the two now overlap
    if !s[0].is_nan() && !s[2].is_nan() && s[2] <= s[1] {
        s[1] = s[1].max(s[3]);
        s[2] = f32::NAN;
        s[3] = f32::NAN;
    }
}

/// What a raster column shows a cell at height `y` looking down (the lower
/// hemisphere's march, `Spec::ground`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Below {
    /// A floor at this height under the cell that is the column's top
    /// surface (ground, roof or a deck's top with nothing over it): sunlit.
    Open(f64),
    /// A floor at this height with a deck over it (ground under a deck, a
    /// lower deck under a higher one).
    Covered(f64),
    /// A building, or a deck's side, rising past the cell: the lower
    /// hemisphere ends here, on what is not open ground.
    Wall,
    /// The terrain rising past the cell (a hillside): it ends here on open ground.
    Hill,
}

/// The column (its height `hq`, building flag and slabs) as seen from height `y`.
#[inline]
fn below(hq: f64, building: bool, slab: &[f32; 4], has_slabs: bool, y: f64) -> Below {
    if hq > y {
        return if building { Below::Wall } else { Below::Hill };
    }
    let mut f = hq;
    let mut covered = false;
    if has_slabs {
        for s in 0..2 {
            let (b, t) = (slab[2 * s] as f64, slab[2 * s + 1] as f64);
            if b.is_nan() {
                continue;
            }
            if t <= y {
                f = f.max(t); // a deck below the cell: its top is the floor
            } else if b > y {
                covered = true; // a deck over the cell: the floor under it is not sunlit
            } else {
                return Below::Wall; // the cell at the deck's own height: its side
            }
        }
    }
    if covered { Below::Covered(f) } else { Below::Open(f) }
}

/// One sector's march over the lower hemisphere for one layer: the highest
/// depression slope (`dy / d`, negative) seen so far, where the present run
/// of open or not-open floor began, whether it is open, and whether a wall
/// or a hill has ended the sector.
#[derive(Clone, Copy)]
struct Down {
    seen: f64,
    run: f64,
    open: bool,
    done: bool,
}

impl Down {
    /// Start from the cell's own column (`own`: what it shows, its floor half a raster cell out).
    fn start(own: Below, y: f64, half_r: f64, acc: &mut [f64; 5], sec: &Sector) -> Self {
        match own {
            Below::Open(f) | Below::Covered(f) => Self { seen: ((f - y) / half_r).min(0.0), run: f64::NEG_INFINITY, open: matches!(own, Below::Open(_)), done: false },
            Below::Hill => {
                open_band(acc, f64::NEG_INFINITY, 0.0, sec);
                Self { seen: 0.0, run: 0.0, open: false, done: true }
            }
            Below::Wall => Self { seen: 0.0, run: 0.0, open: false, done: true },
        }
    }

    /// The column `what` at the step whose inverse distance is `id`.
    #[inline]
    fn step(&mut self, what: Below, y: f64, id: f64, acc: &mut [f64; 5], sec: &Sector) {
        match what {
            Below::Open(f) | Below::Covered(f) => {
                let s = (f - y) * id;
                if s > self.seen {
                    let open = matches!(what, Below::Open(_));
                    if open != self.open {
                        if self.open {
                            open_band(acc, self.run, self.seen, sec);
                        }
                        self.run = self.seen;
                        self.open = open;
                    }
                    self.seen = s;
                }
            }
            Below::Wall => {
                if self.open {
                    open_band(acc, self.run, self.seen, sec);
                }
                self.done = true;
            }
            Below::Hill => {
                open_band(acc, if self.open { self.run } else { self.seen }, 0.0, sec);
                self.done = true;
            }
        }
    }

    /// The sector's end: past the reach the ground counts as open.
    fn finish(&self, acc: &mut [f64; 5], sec: &Sector) {
        if !self.done {
            open_band(acc, if self.open { self.run } else { self.seen }, 0.0, sec);
        }
    }
}

/// Add the open ground between depression slopes `lo` and `hi` (`dy / d`,
/// `lo <= hi <= 0`; `-inf` straight down) of one sector to the irradiance
/// on five surfaces: facing down, +x, -x, +z, -z. `integrate` on the band
/// mirrored into the upper hemisphere (elevations atan(-hi) .. atan(-lo)),
/// in the slopes themselves: sin^2 = t^2 / (1 + t^2), sin 2e = 2t / (1 + t^2)
/// and one arctangent of the band's width.
#[inline]
fn open_band(acc: &mut [f64; 5], lo: f64, hi: f64, s: &Sector) {
    let u = (-hi).max(0.0);
    let (sa2, s2a) = (u * u / (1.0 + u * u), 2.0 * u / (1.0 + u * u));
    let (sb2, s2b, width) = if lo == f64::NEG_INFINITY {
        (1.0, 0.0, FRAC_PI_2 - u.atan())
    } else {
        let v = (-lo).max(u);
        (v * v / (1.0 + v * v), 2.0 * v / (1.0 + v * v), ((v - u) / (1.0 + u * v)).atan())
    };
    if width <= 0.0 {
        return;
    }
    acc[0] += s.dphi * (sb2 - sa2) * 0.5;
    let c = width * 0.5 + (s2b - s2a) * 0.25;
    for k in 0..4 {
        acc[k + 1] += s.faces[k] * c;
    }
}

/// How far the sunlit share's scan looks for what shades a column (metres):
/// a building or a hill shades the ground about its own height away.
pub const SUN_SCAN_M: f64 = 64.0;
/// Azimuths of the sunlit share's scan.
pub const SUN_AZIMUTHS: usize = 8;
/// The horizon's slope (tan of its elevation) under which the sun clears it,
/// and over which it does not, linear between. INFERRED: the sun stands
/// 30-60 deg high through most of a San Francisco day (reference sheet 3.1:
/// noon 29-76 deg over the year), so a horizon under about 31 deg rarely
/// shades the ground and one over about 58 deg nearly always does.
pub const SUN_CLEAR_SLOPE: f64 = 0.6;
pub const SUN_BLOCKED_SLOPE: f64 = 1.6;
/// The radius (metres) round a cell over which the sunlit share of the
/// ground it sees is averaged: its height over the ground, within these.
pub const SUN_DISC_MIN_M: f64 = 4.0;
pub const SUN_DISC_MAX_M: f64 = 32.0;

/// How likely each raster column's top surface is to be in the sun while the
/// sun is up (`Spec::ground`): over `SUN_AZIMUTHS` azimuths, the share of the
/// sun's usual heights its horizon (buildings and the terrain within
/// `SUN_SCAN_M`, not decks: a deck's own shadow is the shader's) lets
/// through. 1 on open ground, about a quarter on a narrow alley's floor.
/// A time-free stand-in for the shadows of the buildings round the street.
struct SunShare {
    g0: [i64; 2],
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl SunShare {
    fn build(raster: &Raster, b: [f64; 4]) -> Self {
        let r = raster.r;
        let g0 = [lattice(b[0], r), lattice(b[1], r)];
        let g1 = [lattice(b[2], r), lattice(b[3], r)];
        let (w, h) = ((g1[0] - g0[0] + 1).max(0) as usize, (g1[1] - g0[1] + 1).max(0) as usize);
        let mut dist = vec![];
        let mut d = r;
        while d <= SUN_SCAN_M {
            dist.push(d);
            d += r.max(d * STEP_GROWTH);
        }
        let dirs: Vec<[f64; 2]> = (0..SUN_AZIMUTHS).map(|k| (k as f64 + 0.5) * TAU / SUN_AZIMUTHS as f64).map(|a| [a.cos(), a.sin()]).collect();
        let mut v = vec![1f32; w * h];
        for j in 0..h {
            for i in 0..w {
                let (x, z) = ((g0[0] + i as i64) as f64 * r, (g0[1] + j as i64) as f64 * r);
                let Some(c) = raster.index(x, z) else { continue };
                let top = raster.height[c] as f64;
                let mut sum = 0.0;
                for dir in &dirs {
                    let mut slope = 0f64;
                    for &d in &dist {
                        let Some(q) = raster.index(x + dir[0] * d, z + dir[1] * d) else { break };
                        slope = slope.max((raster.height[q] as f64 - top) / d);
                    }
                    sum += ((SUN_BLOCKED_SLOPE - slope) / (SUN_BLOCKED_SLOPE - SUN_CLEAR_SLOPE)).clamp(0.0, 1.0);
                }
                v[j * w + i] = (sum / SUN_AZIMUTHS as f64) as f32;
            }
        }
        Self { g0, w, h, v }
    }

    /// At a point (its raster column), 1 off the scanned box.
    fn at(&self, x: f64, z: f64, r: f64) -> f64 {
        let (i, j) = (lattice(x, r) - self.g0[0], lattice(z, r) - self.g0[1]);
        if i < 0 || j < 0 || i as usize >= self.w || j as usize >= self.h {
            return 1.0;
        }
        self.v[j as usize * self.w + i as usize] as f64
    }

    /// The mean over a disc of `radius` round (x, z): its centre and eight points round it.
    fn disc(&self, x: f64, z: f64, radius: f64, r: f64) -> f64 {
        let mut s = self.at(x, z, r);
        for k in 0..8 {
            let a = k as f64 * TAU / 8.0;
            s += self.at(x + radius * a.cos(), z + radius * a.sin(), r);
        }
        s / 9.0
    }
}

/// The march's distances for a raster cell `r`: one cell apart near, then growing by `STEP_GROWTH`.
fn steps(r: f64) -> Vec<f64> {
    let mut v = vec![];
    let mut d = r;
    while d <= REACH_M {
        v.push(d);
        d += r.max(d * STEP_GROWTH);
    }
    v
}

/// One sector of the march: its middle direction, its width, and how much
/// of a surface facing +x, -x, +z and -z it faces (the integral over the
/// sector of the positive part of cos and sin of the azimuth).
#[derive(Clone, Copy)]
struct Sector {
    dir: [f64; 2],
    dphi: f64,
    faces: [f64; 4],
}

/// The integral of max(0, f(phi)) over [a, b], f = cos or sin, split at their zeros (multiples of pi/2).
fn positive_part(a: f64, b: f64, f: fn(f64) -> f64, anti: fn(f64) -> f64) -> f64 {
    let mut cuts = vec![a];
    let q = std::f64::consts::FRAC_PI_2;
    let mut k = (a / q).floor() + 1.0;
    while k * q < b {
        cuts.push(k * q);
        k += 1.0;
    }
    cuts.push(b);
    cuts.windows(2).map(|w| if f(0.5 * (w[0] + w[1])) > 0.0 { anti(w[1]) - anti(w[0]) } else { 0.0 }).sum()
}

fn sectors(n: usize, rot: f64) -> Vec<Sector> {
    let dphi = TAU / n as f64;
    (0..n)
        .map(|s| {
            let pa = (s as f64 + rot) * dphi;
            let pb = pa + dphi;
            let pc = pa + 0.5 * dphi;
            let (cos, sin) = (f64::cos as fn(f64) -> f64, f64::sin as fn(f64) -> f64);
            let neg_cos: fn(f64) -> f64 = |x| -x.cos();
            let neg_sin: fn(f64) -> f64 = |x| -x.sin();
            // +x: cos > 0 (antiderivative sin); -x: -cos > 0 (-sin); +z: sin > 0 (-cos); -z: -sin > 0 (cos).
            let faces = [positive_part(pa, pb, cos, sin), positive_part(pa, pb, neg_cos, neg_sin), positive_part(pa, pb, sin, neg_cos), positive_part(pa, pb, neg_sin, cos)];
            Sector { dir: [pc.cos(), pc.sin()], dphi, faces }
        })
        .collect()
}

/// Add the sky between elevations `lo` and `hi` (radians) of one sector to
/// the irradiance on five surfaces: facing up, +x, -x, +z, -z.
#[inline]
fn integrate(acc: &mut [f64; 5], lo: f64, hi: f64, s: &Sector) {
    if hi <= lo {
        return;
    }
    let (sa, sb) = (lo.sin(), hi.sin());
    acc[0] += s.dphi * (sb * sb - sa * sa) * 0.5;
    let c = (hi - lo) * 0.5 + ((2.0 * hi).sin() - (2.0 * lo).sin()) * 0.25;
    for k in 0..4 {
        acc[k + 1] += s.faces[k] * c;
    }
}

/// The visible sky's irradiance on the five surfaces (over pi) to
/// `(a0, ax, ay, az)`: the function linear in the normal (the L1 band's
/// form) that is exact facing up and, on the horizontal, in the mean
/// (`a0`) and each axis's difference (`ax`, `az`). A projection onto the
/// L1 band instead would miss the irradiance's own l = 2 part, up to a
/// quarter of the open sky's: a band of sky near the horizon (under a deck)
/// read about 0.45 for 0.33 seen.
fn coefficients(acc: [f64; 5]) -> [f32; 4] {
    let e = acc.map(|v| v / PI);
    let a0 = 0.25 * (e[1] + e[2] + e[3] + e[4]);
    [a0 as f32, (0.5 * (e[1] - e[2])) as f32, (e[0] - a0) as f32, (0.5 * (e[3] - e[4])) as f32]
}

/// The open ground's irradiance on the five surfaces (facing down, +x, -x,
/// +z, -z; over pi) to `(g0, gx, gy, gz)`: as `coefficients`, exact facing
/// down (`g0 - gy`) and on the horizontal.
fn ground_coefficients(acc: [f64; 5]) -> [f32; 4] {
    let c = coefficients(acc);
    [c[0], c[1], -c[2], c[3]]
}

/// Bake the block with its corner at (`x0`, `z0`) (a multiple of `spec.cell`).
pub fn bake(scene: &Scene, spec: &Spec, x0: f64, z0: f64) -> Block {
    let n = spec.cells;
    let lay = spec.heights.len().min(MAX_LAYERS);
    let na = n + 2 * APRON;
    let cell = spec.cell;
    let rx0 = x0 - APRON as f64 * cell;
    let rz0 = z0 - APRON as f64 * cell;
    let span = na as f64 * cell;
    let raster = Raster::build(scene, spec.raster, [rx0 - REACH_M, rz0 - REACH_M, rx0 + span + REACH_M, rz0 + span + REACH_M]);
    if raster.trivial {
        return Block::open(n, lay, spec.ground);
    }
    let ground = spec.ground;
    // the sunlit share of the ground round every cell (with the ground; the disc's reach past the block).
    let sun = ground.then(|| SunShare::build(&raster, [rx0 - SUN_DISC_MAX_M, rz0 - SUN_DISC_MAX_M, rx0 + span + SUN_DISC_MAX_M, rz0 + span + SUN_DISC_MAX_M]));
    let dist = steps(spec.raster);
    let inv: Vec<f64> = dist.iter().map(|d| 1.0 / d).collect();
    let rots: Vec<Vec<Sector>> = (0..4).map(|k| sectors(spec.azimuths, k as f64 * 0.25)).collect();
    let mut vals = vec![OPEN; na * na * lay];
    let mut gvals = if ground { vec![OPEN_GROUND; na * na * lay] } else { vec![] };
    let mut inside = vec![false; na * na * lay];
    let idx = |i: usize, j: usize, k: usize| (k * na + j) * na + i;
    let has_slabs = !raster.slabs.is_empty();
    // per-layer scratch
    let mut y = [0f64; MAX_LAYERS];
    let mut active = [false; MAX_LAYERS];
    let mut slope = [0f64; MAX_LAYERS];
    let mut acc = [[0f64; 5]; MAX_LAYERS];
    // the lower hemisphere's open ground (with `ground`): its irradiance and each layer's march.
    let mut gacc = [[0f64; 5]; MAX_LAYERS];
    let mut down = [Down { seen: 0.0, run: 0.0, open: false, done: true }; MAX_LAYERS];
    let mut prev: [[Option<(f64, f64)>; 2]; MAX_LAYERS] = [[None; 2]; MAX_LAYERS];
    // the bottom of a slab over the cell itself (its zenith is shut).
    let mut own: [[Option<f64>; 2]; MAX_LAYERS] = [[None; 2]; MAX_LAYERS];
    let half_r = 0.5 * spec.raster;
    let mut spans: Vec<Vec<(f64, f64)>> = vec![vec![]; MAX_LAYERS];
    for j in 0..na {
        let z = rz0 + (j as f64 + 0.5) * cell;
        for i in 0..na {
            let x = rx0 + (i as f64 + 0.5) * cell;
            let g = scene.ground_ref.at(x, z);
            let gt = scene.ground.ground(x, z);
            let Some(ci) = raster.index(x, z) else { continue };
            let (hc, bld) = (raster.height[ci] as f64, raster.building[ci]);
            let slab = if has_slabs { raster.slabs[ci] } else { [f32::NAN; 4] };
            let mut any = false;
            for k in 0..lay {
                y[k] = (g + spec.heights[k]).max(gt + MIN_ABOVE_GROUND_M);
                let mut out = !(bld && y[k] < hc);
                own[k] = [None, None];
                for s in 0..2 {
                    let (b, t) = (slab[2 * s] as f64, slab[2 * s + 1] as f64);
                    if b.is_nan() {
                        continue;
                    }
                    if y[k] >= b && y[k] <= t {
                        out = false;
                    } else if b > y[k] {
                        own[k][s] = Some(b); // overhead: the zenith is shut
                    }
                }
                active[k] = out;
                inside[idx(i, j, k)] = !out;
                any |= out;
            }
            if !any {
                continue;
            }
            // the sectors' turn: a fixed pattern of the cell's global index.
            let gi = (x / cell).floor() as i64;
            let gj = (z / cell).floor() as i64;
            let secs = &rots[(gi + 2 * gj).rem_euclid(4) as usize];
            for a in acc.iter_mut().take(lay) {
                *a = [0.0; 5];
            }
            if ground {
                for a in gacc.iter_mut().take(lay) {
                    *a = [0.0; 5];
                }
            }
            for sec in secs {
                for k in 0..lay {
                    slope[k] = 0.0;
                    spans[k].clear();
                    for s in 0..2 {
                        // a slab over the cell shuts the sky from straight up down to
                        // its soffit half a raster cell out, whatever the first step finds.
                        prev[k][s] = own[k][s].map(|b| {
                            spans[k].push(((b - y[k]) / half_r, f64::INFINITY));
                            (f64::INFINITY, f64::INFINITY)
                        });
                    }
                }
                // the lowest layer whose lower hemisphere is still being marched (lay: none).
                let mut live = lay;
                if ground {
                    // the lower hemisphere starts on the cell's own column, half a raster cell out.
                    for k in 0..lay {
                        down[k] = if active[k] { Down::start(below(hc, bld, &slab, has_slabs, y[k]), y[k], half_r, &mut gacc[k], sec) } else { Down { seen: 0.0, run: 0.0, open: false, done: true } };
                        if !down[k].done {
                            live = live.min(k);
                        }
                    }
                }
                for (step, &d) in dist.iter().enumerate() {
                    let Some(q) = raster.index(x + sec.dir[0] * d, z + sec.dir[1] * d) else { break };
                    let hq = raster.height[q] as f64;
                    let id = inv[step];
                    for k in 0..lay {
                        let s = (hq - y[k]) * id;
                        if s > slope[k] {
                            slope[k] = s;
                        }
                    }
                    if live < lay {
                        let (bq, sq) = (raster.building[q], if has_slabs { raster.slabs[q] } else { [f32::NAN; 4] });
                        if sq[0].is_nan() {
                            // a column with no deck (the common case): its ground or roof is open.
                            for k in live..lay {
                                let dk = &mut down[k];
                                if dk.done {
                                    continue;
                                }
                                if hq > y[k] {
                                    dk.step(if bq { Below::Wall } else { Below::Hill }, y[k], id, &mut gacc[k], sec);
                                    continue;
                                }
                                let s = (hq - y[k]) * id;
                                if s > dk.seen {
                                    if dk.open {
                                        dk.seen = s;
                                    } else {
                                        dk.step(Below::Open(hq), y[k], id, &mut gacc[k], sec);
                                    }
                                }
                            }
                        } else {
                            for k in live..lay {
                                if !down[k].done {
                                    down[k].step(below(hq, bq, &sq, has_slabs, y[k]), y[k], id, &mut gacc[k], sec);
                                }
                            }
                        }
                        while live < lay && down[live].done {
                            live += 1;
                        }
                    }
                    if has_slabs {
                        let sl = raster.slabs[q];
                        for k in 0..lay {
                            if !active[k] {
                                continue;
                            }
                            for s in 0..2 {
                                let (b, t) = (sl[2 * s] as f64, sl[2 * s + 1] as f64);
                                if b.is_nan() {
                                    prev[k][s] = None;
                                    continue;
                                }
                                let (sb, st) = ((b - y[k]) * id, (t - y[k]) * id);
                                let (mut lo, mut hi) = (sb.min(st), sb.max(st));
                                if let Some((pb, pt)) = prev[k][s] {
                                    lo = lo.min(pb).min(pt);
                                    hi = hi.max(pb).max(pt);
                                }
                                spans[k].push((lo, hi));
                                prev[k][s] = Some((sb, st));
                            }
                        }
                    }
                }
                for k in 0..lay {
                    if !active[k] {
                        continue;
                    }
                    let eh = slope[k].max(0.0).atan();
                    if spans[k].is_empty() {
                        integrate(&mut acc[k], eh, FRAC_PI_2, sec);
                        continue;
                    }
                    spans[k].sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut cur = eh;
                    for &(lo, hi) in &spans[k] {
                        let (lo, hi) = (lo.atan(), hi.atan());
                        if hi <= cur {
                            continue;
                        }
                        if lo > cur {
                            integrate(&mut acc[k], cur, lo.min(FRAC_PI_2), sec);
                        }
                        cur = cur.max(hi);
                        if cur >= FRAC_PI_2 {
                            break;
                        }
                    }
                    integrate(&mut acc[k], cur, FRAC_PI_2, sec);
                }
                if ground {
                    for k in 0..lay {
                        if active[k] {
                            down[k].finish(&mut gacc[k], sec);
                        }
                    }
                }
            }
            for k in 0..lay {
                if active[k] {
                    vals[idx(i, j, k)] = coefficients(acc[k]);
                    if let Some(sun) = &sun {
                        // the open ground it sees, as likely to be sunlit as the ground round it.
                        let p = sun.disc(x, z, spec.heights[k].clamp(SUN_DISC_MIN_M, SUN_DISC_MAX_M), spec.raster) as f32;
                        gvals[idx(i, j, k)] = ground_coefficients(gacc[k]).map(|v| v * p);
                    }
                }
            }
        }
    }
    // the tent over the cells in the open (takes out the sectors' turn pattern).
    let mut vals = tent(&vals, &inside, na, lay);
    // the inside filled from the open: rings of neighbours, then from above.
    let mut filled: Vec<bool> = inside.iter().map(|b| !b).collect();
    fill(&mut vals, &mut filled, na, lay, OPEN);
    let crop = |v: &[[f32; 4]]| {
        let mut c = Vec::with_capacity(n * n * lay);
        for k in 0..lay {
            for j in 0..n {
                for i in 0..n {
                    c.push(v[idx(i + APRON, j + APRON, k)]);
                }
            }
        }
        c
    };
    let mut g = vec![];
    if ground {
        let mut gv = tent(&gvals, &inside, na, lay);
        let mut gfilled: Vec<bool> = inside.iter().map(|b| !b).collect();
        // a cell inside a deck's slab takes the open ground of the cell
        // under it first (the soffit's side: the slab's top faces up and
        // takes no bounce), then the fill as the sky's.
        for j in 0..na {
            for i in 0..na {
                for k in 1..lay {
                    let (c, b) = (idx(i, j, k), idx(i, j, k - 1));
                    if !gfilled[c] && gfilled[b] {
                        gv[c] = gv[b];
                        gfilled[c] = true;
                    }
                }
            }
        }
        fill(&mut gv, &mut gfilled, na, lay, OPEN_GROUND);
        g = crop(&gv);
    }
    Block { cells: n, layers: lay, c: crop(&vals), g }
}

/// The 3 x 3 tent over the cells in the open (`inside` false) of an
/// `na` x `na` x `lay` block, cells inside left as they are.
fn tent(vals: &[[f32; 4]], inside: &[bool], na: usize, lay: usize) -> Vec<[f32; 4]> {
    let idx = |i: usize, j: usize, k: usize| (k * na + j) * na + i;
    let mut tent = vals.to_vec();
    for k in 0..lay {
        for j in 0..na {
            for i in 0..na {
                if inside[idx(i, j, k)] {
                    continue;
                }
                let (mut sum, mut wsum) = ([0f64; 4], 0f64);
                for dj in -1i32..=1 {
                    for di in -1i32..=1 {
                        let (ii, jj) = (i as i32 + di, j as i32 + dj);
                        if ii < 0 || jj < 0 || ii >= na as i32 || jj >= na as i32 {
                            continue;
                        }
                        let c = idx(ii as usize, jj as usize, k);
                        if inside[c] {
                            continue;
                        }
                        let w = ((2 - di.abs()) * (2 - dj.abs())) as f64;
                        for (s, v) in sum.iter_mut().zip(vals[c]) {
                            *s += w * v as f64;
                        }
                        wsum += w;
                    }
                }
                tent[idx(i, j, k)] = sum.map(|s| (s / wsum) as f32);
            }
        }
    }
    tent
}

/// Fill the cells not `filled` from the filled ones: `DILATE_RINGS` rings
/// of neighbours, then from above (`top` over the highest filled cell).
fn fill(vals: &mut [[f32; 4]], filled: &mut [bool], na: usize, lay: usize, top: [f32; 4]) {
    let idx = |i: usize, j: usize, k: usize| (k * na + j) * na + i;
    for _ in 0..DILATE_RINGS {
        let before = filled.to_vec();
        let src = vals.to_vec();
        for k in 0..lay {
            for j in 0..na {
                for i in 0..na {
                    let c = idx(i, j, k);
                    if before[c] {
                        continue;
                    }
                    let (mut sum, mut cnt) = ([0f64; 4], 0u32);
                    for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (ii, jj) = (i as i32 + di, j as i32 + dj);
                        if ii < 0 || jj < 0 || ii >= na as i32 || jj >= na as i32 {
                            continue;
                        }
                        let q = idx(ii as usize, jj as usize, k);
                        if before[q] {
                            for (s, v) in sum.iter_mut().zip(src[q]) {
                                *s += v as f64;
                            }
                            cnt += 1;
                        }
                    }
                    if cnt > 0 {
                        vals[c] = sum.map(|s| (s / cnt as f64) as f32);
                        filled[c] = true;
                    }
                }
            }
        }
    }
    for j in 0..na {
        for i in 0..na {
            let mut above: Option<[f32; 4]> = None;
            for k in (0..lay).rev() {
                let c = idx(i, j, k);
                if filled[c] {
                    above = Some(vals[c]);
                } else {
                    vals[c] = above.unwrap_or(top);
                }
            }
        }
    }
}

/// Half-float bits of `v` (round to nearest even), as the GPU's Rgba16Float holds it.
pub fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rounded = (m + half - 1 + ((m >> shift) & 1)) >> shift;
        return sign | rounded as u16;
    }
    let hm = mant >> 13;
    let rest = mant & 0x1fff;
    let mut out = ((e as u32) << 10) | hm;
    if rest > 0x1000 || (rest == 0x1000 && hm & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

/// The value of half-float bits.
pub fn f16_value(b: u16) -> f32 {
    let sign = if b & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = ((b >> 10) & 0x1f) as i32;
    let m = (b & 0x3ff) as f32;
    match e {
        0 => sign * m * (2.0f32).powi(-24),
        31 => {
            if m == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + m / 1024.0) * (2.0f32).powi(e - 15),
    }
}

/// A volume block as the 3D texture takes it: Rgba16Float `(a0, ax, ay, az)`,
/// layer, then z, then x; with the ground baked, its layers `(g0, gx, gy, gz)`
/// follow in the same order (the second half of the bytes).
pub fn volume_payload(b: &Block) -> Vec<u8> {
    let mut out = Vec::with_capacity((b.c.len() + b.g.len()) * 8);
    for v in b.c.iter().chain(&b.g) {
        for x in v {
            out.extend_from_slice(&f16_bits(*x).to_le_bytes());
        }
    }
    out
}

/// The plane's reach from the up-facing visibility at `PLANE_HEIGHTS`: the
/// height at which it reaches `PLANE_OPEN_SHARE` (linear between the heights;
/// the last height's further when it never does, up to the reach).
pub fn plane_reach(up: &[f64]) -> f64 {
    let hs = PLANE_HEIGHTS;
    if up[0] >= PLANE_OPEN_SHARE {
        return hs[0];
    }
    for k in 1..up.len().min(hs.len()) {
        if up[k] >= PLANE_OPEN_SHARE {
            let t = ((PLANE_OPEN_SHARE - up[k - 1]) / (up[k] - up[k - 1]).max(1e-9)).clamp(0.0, 1.0);
            return hs[k - 1] + (hs[k] - hs[k - 1]) * t;
        }
    }
    REACH_M.max(hs[hs.len() - 1])
}

/// A plane block as the plane texture takes it, cropped to the plane: per
/// cell Rgba16Float `(a0, ay, ground, reach)` (street level), z then x.
/// Returns the texel box (x, z, width, height) and the bytes.
pub fn plane_payload(b: &Block, ground: &GroundRef, block: (i32, i32)) -> ((u32, u32, u32, u32), Vec<u8>) {
    let m = ground.map;
    let (bx, bz) = (block.0.max(0) as usize * PLANE_TILE_CELLS, block.1.max(0) as usize * PLANE_TILE_CELLS);
    let w = PLANE_TILE_CELLS.min(m.w.saturating_sub(bx));
    let h = PLANE_TILE_CELLS.min(m.h.saturating_sub(bz));
    let mut out = Vec::with_capacity(w * h * 8);
    for j in 0..h {
        for i in 0..w {
            let up: Vec<f64> = (0..b.layers).map(|k| visibility(b.at(i, j, k), [0.0, 1.0, 0.0])).collect();
            let c = b.at(i, j, 0);
            let texel = [c[0], c[2], ground.h[(bz + j) * m.w + bx + i], plane_reach(&up) as f32];
            for x in texel {
                out.extend_from_slice(&f16_bits(x).to_le_bytes());
            }
        }
    }
    ((bx as u32, bz as u32, w as u32, h as u32), out)
}

/// The plane before any block is baked: the open sky over its ground (`(0.5, 0.5, ground, 1)`), z then x.
pub fn plane_initial(ground: &GroundRef) -> Vec<u8> {
    let mut out = Vec::with_capacity(ground.h.len() * 8);
    for g in &ground.h {
        for x in [0.5f32, 0.5, *g, 1.0] {
            out.extend_from_slice(&f16_bits(x).to_le_bytes());
        }
    }
    out
}

/// The cache folder name of one level (or the plane): this file's source,
/// the version, the scene (`scene_key`: occluders, ground, ground scale,
/// structures) and the spec.
pub fn level_key(scene_key: &[u8; 32], spec: &Spec) -> String {
    let mut h = Sha256::new();
    h.update(b"eo-ambient");
    h.update(BAKE_VERSION.to_le_bytes());
    h.update(Sha256::digest(BAKE_SOURCE.as_bytes()));
    h.update(scene_key);
    h.update(spec.key_bytes());
    for v in [REACH_M, MIN_ABOVE_GROUND_M, MIN_DECK_SLAB_M, STEP_GROWTH, PLANE_OPEN_SHARE] {
        h.update(v.to_le_bytes());
    }
    h.update([APRON as u8, DILATE_RINGS as u8]);
    let d = h.finalize();
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `<dir>/ambient/<key>/<x>_<z>.eoav`.
pub fn cache_path(dir: &std::path::Path, key: &str, at: (i32, i32)) -> std::path::PathBuf {
    dir.join("ambient").join(key).join(format!("{}_{}.eoav", at.0, at.1))
}

/// A cache file: magic, version, the block's place and size, the payload, and the SHA-256 of all before it.
pub fn encode_cache(at: (i32, i32), cells: u32, layers: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(payload.len() + 64);
    b.extend_from_slice(CACHE_MAGIC);
    b.extend_from_slice(&BAKE_VERSION.to_le_bytes());
    b.extend_from_slice(&at.0.to_le_bytes());
    b.extend_from_slice(&at.1.to_le_bytes());
    b.extend_from_slice(&cells.to_le_bytes());
    b.extend_from_slice(&layers.to_le_bytes());
    b.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    b.extend_from_slice(payload);
    let d = Sha256::digest(&b);
    b.extend_from_slice(&d);
    b
}

/// The payload of a cache file, if it is whole and is the block asked for.
pub fn decode_cache(bytes: &[u8], at: (i32, i32), cells: u32, layers: u32) -> Option<Vec<u8>> {
    if bytes.len() < 32 + 32 || &bytes[0..4] != CACHE_MAGIC {
        return None;
    }
    let (body, digest) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap());
    let i = |o: usize| i32::from_le_bytes(body[o..o + 4].try_into().unwrap());
    if u(4) != BAKE_VERSION || (i(8), i(12)) != at || u(16) != cells || u(20) != layers {
        return None;
    }
    let len = u64::from_le_bytes(body[24..32].try_into().unwrap()) as usize;
    (body.len() == 32 + len).then(|| body[32..].to_vec())
}

/// Write `bytes` to `path` through a temporary file and a rename.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, z0: f64, x1: f64, z1: f64) -> Vec<[f64; 2]> {
        vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]]
    }

    fn scene(prisms: Vec<Prism>, decks: Vec<DeckPiece>) -> Scene {
        let map = PlaneMap { min: [-1024.0, -1024.0], w: 256, h: 256 };
        Scene::new(prisms, decks, Arc::new(FlatGround(0.0)), Arc::new(GroundRef::flat(map, 0.0)))
    }

    /// A courtyard: a square ring of 20 m buildings (60 m outside, 20 m yard) centred on (64, 64).
    fn courtyard() -> Scene {
        let p = |r: Vec<[f64; 2]>| Prism { ring: r, top: 20.0 };
        scene(vec![p(rect(34.0, 34.0, 94.0, 54.0)), p(rect(34.0, 74.0, 94.0, 94.0)), p(rect(34.0, 54.0, 54.0, 74.0)), p(rect(74.0, 54.0, 94.0, 74.0))], vec![])
    }

    const UP: [f64; 3] = [0.0, 1.0, 0.0];

    #[test]
    fn half_floats_round_trip() {
        for v in [0.0f32, 0.5, 1.0, -0.25, 12.375, 282.5, 1e-5, -3.0e-6, 65504.0] {
            let b = f16_bits(v);
            let back = f16_value(b);
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {b:#06x} -> {back}");
            assert_eq!(f16_bits(back), b, "{v}: a half float is its own rounding");
        }
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
    }

    #[test]
    fn open_ground_bakes_to_the_open_sky() {
        let s = scene(vec![], vec![]);
        let b = bake(&s, &Spec::volume(4.0), 0.0, 0.0);
        assert!(b.c.iter().all(|c| *c == OPEN));
        // a far building (beyond the reach) changes nothing either.
        let far = scene(vec![Prism { ring: rect(400.0, 0.0, 440.0, 40.0), top: 80.0 }], vec![]);
        let b = bake(&far, &Spec::volume(4.0), 0.0, 0.0);
        for c in &b.c {
            for k in 0..4 {
                assert!((c[k] - OPEN[k]).abs() < 1e-6, "{c:?}");
            }
        }
    }

    #[test]
    fn a_courtyard_is_darker_at_the_bottom_than_an_open_square_and_symmetric() {
        let s = courtyard();
        let spec = Spec::volume(4.0);
        let b = bake(&s, &spec, 0.0, 0.0);
        let open = bake(&scene(vec![], vec![]), &spec, 0.0, 0.0);
        // the yard's middle cells (centres 62 and 66 m), street level.
        let yard = b.at(15, 15, 0);
        let up = visibility(yard, UP);
        let open_up = visibility(open.at(15, 15, 0), UP);
        println!("yard bottom {yard:?}: up {up:.3} (open {open_up:.3}); at 18 m {:.3}, 31 m {:.3}", visibility(b.at(15, 15, 8), UP), visibility(b.at(15, 15, 15), UP));
        assert!((open_up - 1.0).abs() < 1e-6);
        assert!(up < 0.5 * open_up, "the yard's floor sees {up} of the sky (open {open_up})");
        // brighter higher up the yard, open over the roofs.
        assert!(visibility(b.at(15, 15, 8), UP) > up + 0.05);
        assert!(visibility(b.at(15, 15, 15), UP) > 0.9);
        // symmetric: mirror cells across the yard's middle agree (x and z mirrored, and swapped).
        // (cells 14-17: centres 58-70 m, clear of the walls at 54 and 74 m,
        // whose raster cells fall to one side of the line.)
        let mut worst = 0f64;
        for k in [0usize, 4, 9] {
            for j in 14..=17 {
                for i in 14..=17 {
                    let (mi, mj) = (31 - i, 31 - j);
                    let a = b.at(i, j, k);
                    let c = b.at(mi, mj, k);
                    let t = b.at(j, i, k);
                    worst = worst.max((a[0] - c[0]).abs() as f64).max((a[2] - c[2]).abs() as f64);
                    worst = worst.max((a[1] + c[1]).abs() as f64).max((a[3] + c[3]).abs() as f64);
                    worst = worst.max((a[0] - t[0]).abs() as f64).max((a[1] - t[3]).abs() as f64);
                }
            }
        }
        assert!(worst < 0.03, "the symmetric yard bakes {worst} out of symmetry");
        // its centre has no lean.
        assert!(yard[1].abs() < 0.02 && yard[3].abs() < 0.02, "{yard:?}");
    }

    #[test]
    fn a_cell_under_a_deck_loses_the_sky_above_but_keeps_the_sides() {
        // a deck 20 m wide, its soffit 9 m and its top 10 m up, running along x through z = 64.
        let deck = DeckPiece { a: [-400.0, 64.0], b: [600.0, 64.0], half: [10.0, 10.0], top: [10.0, 10.0], bottom: [9.0, 9.0] };
        let s = scene(vec![], vec![deck]);
        let b = bake(&s, &Spec::volume(4.0), 0.0, 0.0);
        let under = b.at(16, 15, 0); // centre (66, 62): under the deck, 1 m up
        let side_n = [0.0, 0.0, 1.0]; // facing across the deck (toward its edge)
        let along = [1.0, 0.0, 0.0];
        let (up, side, len) = (visibility(under, UP), visibility(under, side_n), visibility(under, along));
        println!("under the deck {under:?}: up {up:.3}, across {side:.3}, along {len:.3}");
        // (a function linear in the normal cannot tell a wall facing across the
        // deck from one facing along it: their pairs share `a0`; the open sides
        // show as the share of the sideways sky kept against the share of the sky above.)
        assert!(up < 0.35, "under the deck the floor still sees {up} of the sky");
        assert!(side > 0.2 && len > 0.2, "under the deck the sides are shut ({side}, {len}; open 0.5)");
        assert!(side / 0.5 > up + 0.15, "the sides keep {} of their sky, no more than the top's {up}", side / 0.5);
        // above the deck: open again; inside the slab: filled from the open (never black).
        assert!(visibility(b.at(16, 15, 6), UP) > 0.95, "over the deck {:?}", b.at(16, 15, 6));
        assert!(visibility(b.at(16, 15, 4), UP) > 0.0);
        // beside the deck, past its edge, the sky comes back.
        let beside = b.at(16, 23, 0); // z = 94: 20 m from the edge
        assert!(visibility(beside, UP) > up + 0.3, "beside {beside:?}");
    }

    #[test]
    fn tiles_meet_without_a_seam() {
        // a street of buildings across the tile border at x = 128.
        let mut prisms = vec![];
        for k in 0..6 {
            let x = 60.0 + k as f64 * 25.0;
            prisms.push(Prism { ring: rect(x, 20.0, x + 18.0, 50.0), top: 12.0 + 7.0 * k as f64 });
            prisms.push(Prism { ring: rect(x, 66.0, x + 18.0, 100.0), top: 30.0 - 3.0 * k as f64 });
        }
        let s = scene(prisms, vec![]);
        let spec = Spec::volume(4.0);
        let a = bake(&s, &spec, 0.0, 0.0);
        let b = bake(&s, &spec, 128.0, 0.0);
        let mid = bake(&s, &spec, 64.0, 0.0); // straddles the border
        let mut worst = 0f32;
        for k in 0..spec.heights.len() {
            for j in 0..32 {
                for i in 0..32 {
                    let m = mid.at(i, j, k);
                    let t = if i < 16 { a.at(i + 16, j, k) } else { b.at(i - 16, j, k) };
                    for c in 0..4 {
                        worst = worst.max((m[c] - t[c]).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "a block straddling the border differs from the tiles by {worst}");
        // and across the border the field moves no more than between any two neighbours inside a tile.
        let step = |x: &Block, y: &Block, i0: usize, i1: usize, j: usize, k: usize| {
            let (p, q) = (x.at(i0, j, k), y.at(i1, j, k));
            (0..4).map(|c| (p[c] - q[c]).abs()).fold(0f32, f32::max)
        };
        let (mut across, mut within) = (0f32, 0f32);
        for k in 0..spec.heights.len() {
            for j in 0..32 {
                across = across.max(step(&a, &b, 31, 0, j, k));
                for i in 0..31 {
                    within = within.max(step(&a, &a, i, i + 1, j, k)).max(step(&b, &b, i, i + 1, j, k));
                }
            }
        }
        println!("largest step across the border {across:.4}, between neighbours inside a tile {within:.4}");
        assert!(across <= within + 1e-4, "a seam: {across} across the border against {within} inside");
    }

    #[test]
    fn bakes_are_the_same_on_any_thread_and_through_the_cache() {
        let s = Arc::new(courtyard());
        let spec = Spec::volume(4.0);
        let tiles: Vec<(i32, i32)> = vec![(0, 0), (-1, 0), (0, 1), (1, 1)];
        let one: Vec<Vec<u8>> = tiles.iter().map(|t| volume_payload(&bake(&s, &spec, t.0 as f64 * TILE_M, t.1 as f64 * TILE_M))).collect();
        let many: Vec<Vec<u8>> = std::thread::scope(|sc| {
            let hs: Vec<_> = tiles
                .iter()
                .map(|t| {
                    let (s, spec) = (s.clone(), spec.clone());
                    let t = *t;
                    sc.spawn(move || volume_payload(&bake(&s, &spec, t.0 as f64 * TILE_M, t.1 as f64 * TILE_M)))
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(one, many, "a tile baked on another thread differs");
        for (t, p) in tiles.iter().zip(&one) {
            let file = encode_cache(*t, 32, LAYERS as u32, p);
            assert_eq!(decode_cache(&file, *t, 32, LAYERS as u32).as_deref(), Some(p.as_slice()), "the cache does not give back what it was given");
            assert!(decode_cache(&file, (t.0 + 1, t.1), 32, LAYERS as u32).is_none(), "a file read for another tile");
            let mut bad = file.clone();
            bad[40] ^= 1;
            assert!(decode_cache(&bad, *t, 32, LAYERS as u32).is_none(), "a damaged file read");
        }
        // the plane too.
        let pspec = Spec::plane();
        let p1 = volume_payload(&bake(&s, &pspec, -1024.0, -1024.0));
        let p2 = std::thread::scope(|sc| sc.spawn(|| volume_payload(&bake(&s, &pspec, -1024.0, -1024.0))).join().unwrap());
        assert_eq!(p1, p2);
    }

    #[test]
    fn the_plane_reach_is_where_the_sky_opens() {
        assert_eq!(plane_reach(&[0.99, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]), 1.0);
        let r = plane_reach(&[0.3, 0.4, 0.5, 0.7, 0.9, 1.0, 1.0]);
        assert!(r > 32.0 && r < 64.0, "{r}");
        assert_eq!(plane_reach(&[0.1; 7]), REACH_M);
        // the courtyard's plane block: dark in the yard, open far from it, the reach about the roofs.
        let s = courtyard();
        let b = bake(&s, &Spec::plane(), 0.0, 0.0); // the map's block (1, 1)
        let ground = GroundRef::flat(PlaneMap { min: [-1024.0, -1024.0], w: 256, h: 256 }, 0.0);
        let (bx, bytes) = plane_payload(&b, &ground, (1, 1));
        assert_eq!(bx, (128, 128, 128, 128));
        let texel = |i: usize, j: usize| {
            let o = (j * PLANE_TILE_CELLS + i) * 8;
            [0, 1, 2, 3].map(|c| f16_value(u16::from_le_bytes([bytes[o + 2 * c], bytes[o + 2 * c + 1]])))
        };
        let yard = texel(7, 7); // centre (60, 60), in the yard
        let open = texel(100, 100); // (804, 804)
        println!("plane: yard {yard:?}, open {open:?}");
        assert!(yard[0] + yard[1] < 0.6 && (open[0] + open[1] - 1.0).abs() < 1e-3);
        assert!(yard[3] > 10.0 && yard[3] < 64.0, "the yard's reach {}", yard[3]);
    }

    const DOWN: [f64; 3] = [0.0, -1.0, 0.0];

    /// The deck of `a_cell_under_a_deck_loses_the_sky_above_but_keeps_the_sides`:
    /// 20 m wide (z 54-74), its soffit 9 m and its top 10 m up, along x through z = 64.
    fn deck() -> DeckPiece {
        DeckPiece { a: [-400.0, 64.0], b: [600.0, 64.0], half: [10.0, 10.0], top: [10.0, 10.0], bottom: [9.0, 9.0] }
    }

    #[test]
    fn open_ground_bakes_to_open_ground_and_the_sky_is_unchanged_by_it() {
        let open = bake(&scene(vec![], vec![]), &Spec::volume(4.0).with_ground(true), 0.0, 0.0);
        assert!(open.c.iter().all(|c| *c == OPEN) && open.g.iter().all(|g| *g == OPEN_GROUND));
        assert!((ground_share(OPEN_GROUND, DOWN) - 1.0).abs() < 1e-9 && ground_share(OPEN_GROUND, UP) == 0.0);
        assert!((ground_share(OPEN_GROUND, [1.0, 0.0, 0.0]) - 0.5).abs() < 1e-9);
        // a scene that is not trivial: the sky's bake is bit for bit the same with the ground baked beside it.
        let mut s = courtyard();
        s = Scene::new(s.prisms.clone(), vec![deck()], s.ground.clone(), s.ground_ref.clone());
        let spec = Spec::volume(4.0);
        let (without, with) = (bake(&s, &spec, 0.0, 0.0), bake(&s, &spec.clone().with_ground(true), 0.0, 0.0));
        assert!(without.g.is_empty() && with.g.len() == with.c.len());
        assert_eq!(without.c, with.c, "the sky changed with the ground baked");
        assert_ne!(spec.key_bytes(), spec.clone().with_ground(true).key_bytes(), "the ground has its own cache key");
        // the payload: the sky's layers, then the ground's.
        let p = volume_payload(&with);
        assert_eq!(p.len(), 2 * volume_payload(&without).len());
        assert_eq!(p[..p.len() / 2], volume_payload(&without)[..]);
        // open ground far from everything (the reach's edge of the block) is open ground within a hair.
        let far = bake(&scene(vec![Prism { ring: rect(250.0, 0.0, 260.0, 10.0), top: 6.0 }], vec![]), &spec.clone().with_ground(true), 0.0, 0.0);
        let g = far.ground_at(2, 30, 0);
        assert!((ground_share(g, DOWN) - 1.0).abs() < 1e-3 && ground_share(g, [-1.0, 0.0, 0.0]) > 0.49, "{g:?}");
    }

    #[test]
    fn a_slab_over_open_ground_takes_the_bounce_underneath_brighter_towards_its_edge_and_none_on_top() {
        let s = scene(vec![], vec![deck()]);
        let b = bake(&s, &Spec::volume(4.0).with_ground(true), 0.0, 0.0);
        // cells 1.5-2 m under the soffit (layer 3: 7 m up): the middle (z 62 and 66), 4 m in from
        // the edge (z 58), and 6 m beside the deck (z 50 and 78).
        let down = |j: usize| ground_share(b.ground_at(16, j, 3), DOWN);
        let (middle, middle2, in_edge, beside, beside2) = (down(15), down(16), down(14), down(12), down(19));
        // just over the deck (layer 5: 11 m): it faces up, so none.
        let top = b.ground_at(16, 15, 5);
        println!("open ground facing down under the deck: middle {middle:.3} / {middle2:.3}, 4 m in from the edge {in_edge:.3}, 6 m beside {beside:.3} / {beside2:.3}; over the deck down {:.3}, up {:.3}", ground_share(top, DOWN), ground_share(top, UP));
        assert!((middle - middle2).abs() < 0.02, "the deck is symmetric: {middle} {middle2}");
        assert!(middle > 0.1, "under the deck's middle the soffit still sees the street beside it: {middle}");
        assert!(in_edge > middle + 0.05, "brighter towards the open edge: {in_edge} against {middle}");
        assert!(beside > in_edge + 0.1 && (beside - beside2).abs() < 0.02, "beside the deck: {beside} {beside2}");
        assert!(beside < 0.95, "beside the deck the ground under it is shaded: {beside}");
        assert_eq!(ground_share(top, UP), 0.0, "an up-facing surface takes no bounce");
        assert!(ground_share(top, DOWN) > 0.95, "over the deck its top is open (sunlit): {}", ground_share(top, DOWN));
        // a wall facing the deck from beside it (+z facing -z) sees the street and the covered ground under it.
        let wall = ground_share(b.ground_at(16, 21, 0), [0.0, 0.0, -1.0]);
        assert!(wall > 0.3 && wall <= 0.5, "a wall beside the deck: {wall}");
    }

    #[test]
    fn a_slab_over_a_closed_courtyard_takes_much_less_than_over_open_ground() {
        // the courtyard (60 m ring of 20 m blocks, yard 54-74) roofed by a slab at 9-10 m over the yard.
        let lid = DeckPiece { a: [54.0, 64.0], b: [74.0, 64.0], half: [10.0, 10.0], top: [10.0, 10.0], bottom: [9.0, 9.0] };
        let c = courtyard();
        let yard = Scene::new(c.prisms.clone(), vec![lid], c.ground.clone(), c.ground_ref.clone());
        let spec = Spec::volume(4.0).with_ground(true);
        let closed = bake(&yard, &spec, 0.0, 0.0);
        let open = bake(&scene(vec![], vec![deck()]), &spec, 0.0, 0.0);
        let (gc, go) = (ground_share(closed.ground_at(15, 15, 3), DOWN), ground_share(open.ground_at(16, 15, 3), DOWN));
        println!("facing down 2 m under the slab: over the closed yard {gc:.3}, over open ground {go:.3}");
        assert!(gc < 0.25 * go, "the closed yard's slab takes {gc} against {go} over open ground");
    }

    #[test]
    fn a_narrow_alleys_shaded_floor_bounces_little_onto_its_walls() {
        // an 8 m alley (x 60-68) between two 25 m blocks, 128 m long.
        let s = scene(vec![Prism { ring: rect(20.0, 0.0, 60.0, 128.0), top: 25.0 }, Prism { ring: rect(68.0, 0.0, 108.0, 128.0), top: 25.0 }], vec![]);
        let spec = Spec::volume(4.0).with_ground(true);
        let b = bake(&s, &spec, 0.0, 0.0);
        // 2 m in front of the west wall, 1 m up, facing across the alley; and the same wall facing a plaza.
        let east = [1.0, 0.0, 0.0];
        let alley = ground_share(b.ground_at(15, 16, 0), east);
        let open = ground_share(bake(&scene(vec![], vec![]), &spec, 0.0, 0.0).ground_at(15, 16, 0), east);
        // high up the alley its walls see the other wall, not the floor.
        let up_high = ground_share(b.ground_at(15, 16, 10), east);
        println!("a wall's open, sunlit ground: in the alley {alley:.3} (21 m up {up_high:.3}), facing open ground {open:.3}");
        assert!((open - 0.5).abs() < 1e-6);
        assert!(alley < 0.4 * open, "the alley's floor lies in its walls' shadow: {alley} against {open}");
        assert!(alley > 0.0 && up_high < alley, "{alley} {up_high}");
    }

    #[test]
    fn the_open_ground_meets_without_a_seam() {
        // a deck and buildings across the tile border at x = 128.
        let mut prisms = vec![];
        for k in 0..6 {
            let x = 60.0 + k as f64 * 25.0;
            prisms.push(Prism { ring: rect(x, 20.0, x + 18.0, 40.0), top: 12.0 + 7.0 * k as f64 });
        }
        let d = DeckPiece { a: [0.0, 70.0], b: [256.0, 90.0], half: [9.0, 11.0], top: [11.0, 14.0], bottom: [9.5, 12.0] };
        let s = scene(prisms, vec![d]);
        let spec = Spec::volume(4.0).with_ground(true);
        let (a, b, mid) = (bake(&s, &spec, 0.0, 0.0), bake(&s, &spec, 128.0, 0.0), bake(&s, &spec, 64.0, 0.0));
        let mut worst = 0f32;
        for k in 0..spec.heights.len() {
            for j in 0..32 {
                for i in 0..32 {
                    let m = mid.ground_at(i, j, k);
                    let t = if i < 16 { a.ground_at(i + 16, j, k) } else { b.ground_at(i - 16, j, k) };
                    for c in 0..4 {
                        worst = worst.max((m[c] - t[c]).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "a block straddling the border differs from the tiles by {worst}");
    }

    #[test]
    fn layers_climb_as_documented() {
        assert_eq!(layer_centre(0), 1.0);
        assert_eq!(layer_centre(FINE_LAYERS - 1), 31.0);
        assert_eq!(layer_centre(FINE_LAYERS), 35.0);
        assert_eq!(layer_centre(LAYERS - 1), TOP_M - 3.0);
        assert_eq!(TOP_M, 80.0);
        assert!(steps(2.0).len() < 60 && steps(2.0).last().unwrap() <= &REACH_M);
    }
}
