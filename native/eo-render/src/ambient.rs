//! Large-scale sky visibility (`LightingOptions::ambient_volume`: 1 high,
//! 2 hyper): narrow streets, courtyards and the ground under a deck get less
//! of the sky's light than an open plaza. The bake (ambient_bake.rs) runs on
//! worker threads per 128 m tile, nearest the camera first, through a cache
//! (`<cache>/ambient/<key>/<x>_<z>.eoav`); the results go through this
//! module's own `UploadQueue` into a staging buffer and are copied into the
//! textures in `encode` (timed in `lighting::Q_AMBIENT_UPLOAD`):
//!
//! * binding 13, the volume: a 3D texture of `N` x `N` cells a level and
//!   `LAYERS` terrain-following layers, one slab of layers per level (1: one
//!   level of 4 m cells, 6 x 6 tiles, at least 320 m round the camera; 2:
//!   4 m cells over 8 x 8 tiles, at least 430 m, and 2 m cells over 4 x 4,
//!   at least 170 m). A level is a ring: tile (x, z) lives at
//!   (x mod ring, z mod ring), so the shader reads it with the repeat sampler
//!   straight from world x and z, and re-centring rewrites only the tiles that
//!   change. Cell `(a0, ax, ay, az)` (ambient_bake.rs).
//! * binding 14, the citywide plane: 8 m cells over the city, `(a0, ay,
//!   ground, reach)` at street level, baked once in 1,024 m blocks and
//!   cached; its ground is the terrain-following grid's floor everywhere.
//!
//! `Look.lx[10]` holds each level's resident rectangle of tiles (two tile
//! indices packed in a float each, `pack`), `lx[11]` the plane's corner and
//! cell (0: nothing resident, the feature inert). A tile enters the
//! rectangle only once its copy is encoded; a re-centred ring drops the
//! tiles leaving it at once and adds the new ones as they land, so the shader
//! never reads a slot being rewritten.
//!
//! With the ground bounce (`LightingOptions::ground_bounce_on`) the levels
//! also hold the open ground's share of the lower hemisphere
//! (ambient_bake.rs `Spec::ground`): the volume's depth doubles, the second
//! half holding each level's ground layers in the same order (so the shader
//! reads them half the texture's depth further), each tile written as two
//! parts (sky, then ground: its two halves, by byte offset). `Look.lx[0]`
//! y z w carry the sunlit ground's exitance from the sun (eo-look
//! `bounce::sun_exitance`; zero while the bounce is off). The citywide plane
//! holds no ground: past the levels there is no bounce.

use crate::ambient_bake::{self as bake, Block, GroundRef, PlaneMap, Spec};
use crate::camera::Camera;
use crate::lighting::{self, Ts};
use crate::look::{LX_AMBIENT, LX_FLAGS, LookGpu};
use crate::upload::{Data, Job, Target, UploadQueue, Write};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Bytes the volume and the plane may write a frame (the queue's budget):
/// under the 1 MiB that moves the scene probe's face to a later frame
/// (`PROBE_DEFER_UPLOAD_BYTES`), so the volume alone never defers it.
pub const UPLOAD_BUDGET: u64 = 1_000_000;
/// The staging buffer the writes land in before `encode` copies them: room
/// for a frame's budget and the largest single write (one 2 m tile, 768 KiB).
pub const STAGING_BYTES: u64 = 2 << 20;
/// A level re-centres once the camera is this far past the half tile round
/// its ring's middle corner (hysteresis, metres).
pub const RECENTRE_SLACK_M: f64 = 16.0;
/// A plane block ranks as a volume tile this much further away (the volume first).
pub const PLANE_RANK_M: f64 = 400.0;
/// Horizontal fade from a level to the one under it (or the plane) inside its rectangle's edge, metres.
pub const EDGE_FADE_M: f64 = 48.0;
/// A shaded point reads the volume this far along its geometric normal (half
/// a 4 m cell: a wall reads the street before it, a roof the air over it).
pub const NORMAL_OFFSET_M: f64 = 1.5;
/// Over the top layer the volume eases to the open sky by the plane's reach, at least over this height.
pub const OPEN_RISE_M: f64 = 16.0;
/// The radiance of what blocks the sky, as a share of the rig's ground lobe
/// (the sunlit street's bounce): the blocked part of the sky term is lit by
/// it instead of left black. INFERRED: facades and soffits stand in the
/// same sun and sky as the street they face, at about its albedo or above
/// (reference sheet 2.1: weathered concrete 0.25-0.35 against aged asphalt
/// 0.15), and receive about half of its irradiance (vertical, partly
/// shaded); 1.0 keeps a canyon's floor at about half an open square's fill
/// and a soffit's never black (5.10, Selby St under I-280).
pub const OCCLUDER_OVER_GROUND: f64 = 1.0;
/// The ground bounce reads an underside's open ground shifted toward the sun
/// by its height x cot(the sun's altitude) (its deck's shadow, ambient.wgsl
/// `amb_ground`), the cotangent at most this (the sun 14 deg high) and the
/// shift at most `SUN_SHIFT_MAX_M`: INFERRED, keeping the read near the deck
/// at a low sun, when the bounce is weak anyway (the sun's irradiance on the
/// ground goes with the sine of its altitude).
pub const SUN_SHIFT_COT_MAX: f64 = 4.0;
pub const SUN_SHIFT_MAX_M: f64 = 32.0;

/// A level of the volume: its cell, its ring of tiles a side, its slab in the texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub cell: f64,
    pub ring: usize,
    pub slab: usize,
}

impl Level {
    /// Cells a tile side.
    pub fn cells(&self) -> usize {
        (bake::TILE_M / self.cell).round() as usize
    }
    /// Texels a side of the ring.
    pub fn texels(&self) -> usize {
        self.ring * self.cells()
    }
    /// Metres from the camera to the ring's edge, at least (the camera within the half tile and the slack of the middle corner).
    pub fn reach_m(&self) -> f64 {
        self.ring as f64 * 0.5 * bake::TILE_M - 0.5 * bake::TILE_M - RECENTRE_SLACK_M
    }
}

/// The levels of `ambient_volume` (0 none; 1 high; 2 hyper: the far level first).
pub fn levels(option: u8) -> Vec<Level> {
    match option {
        0 => vec![],
        1 => vec![Level { cell: 4.0, ring: 8, slab: 0 }],
        _ => vec![Level { cell: 4.0, ring: 8, slab: 0 }, Level { cell: 2.0, ring: 4, slab: 1 }],
    }
}

/// The buildings the bake reads: the loaded city's, or rings and heights (tests).
pub enum AmbientBuildings {
    City(Arc<eo_data::City>),
    Rings(Vec<(Vec<[f64; 2]>, f64)>),
}

/// The ground under them: the city's elevation grid, or flat (tests).
pub enum AmbientGround {
    Elevation(Arc<eo_data::ElevationGrid>),
    Flat(f64),
}

/// What the bake reads, from the loaded world (eo-app `make_renderer`).
pub struct AmbientInputs {
    /// The buildings' rings and heights (game frame).
    pub buildings: AmbientBuildings,
    /// The construction lift the drawn shells stand on (eo-assets placement).
    pub lift: f64,
    pub ground: AmbientGround,
    /// The grade-separated roads, with `--structures` (their decks occlude); None without.
    pub structures: Option<Arc<eo_structures::StructureWorld>>,
    /// Decks besides the structures' (tests).
    pub decks: Vec<bake::DeckPiece>,
    /// The cache root (`--cache`), None with `--no-cache`.
    pub cache_dir: Option<PathBuf>,
    /// Bake workers.
    pub threads: usize,
}

impl AmbientInputs {
    /// The loaded city's buildings on its elevation grid, with its structures' decks.
    pub fn city(city: Arc<eo_data::City>, lift: f64, elevation: Arc<eo_data::ElevationGrid>, structures: Option<Arc<eo_structures::StructureWorld>>, cache_dir: Option<PathBuf>, threads: usize) -> Self {
        Self { buildings: AmbientBuildings::City(city), lift, ground: AmbientGround::Elevation(elevation), structures, decks: vec![], cache_dir, threads }
    }

    /// Each building's ring and height.
    fn rings(&self) -> Vec<(Vec<[f64; 2]>, f64)> {
        match &self.buildings {
            AmbientBuildings::City(c) => c.buildings.iter().map(|b| (b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), b.height as f64)).collect(),
            AmbientBuildings::Rings(r) => r.clone(),
        }
    }

    /// The occluders' bounds (min x, min z, max x, max z), None without any.
    fn bounds(&self) -> Option<[f64; 4]> {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut add = |x: f64, z: f64| b = [b[0].min(x), b[1].min(z), b[2].max(x), b[3].max(z)];
        match &self.buildings {
            AmbientBuildings::City(c) => c.buildings.iter().flat_map(|bd| &bd.ring).for_each(|p| add(p[0] as f64, p[1] as f64)),
            AmbientBuildings::Rings(r) => r.iter().flat_map(|(ring, _)| ring).for_each(|p| add(p[0], p[1])),
        }
        for d in &self.decks {
            add(d.a[0], d.a[1]);
            add(d.b[0], d.b[1]);
        }
        (b[0] <= b[2]).then_some(b)
    }

    /// The ground as the bake reads it, and its digest for the cache key.
    fn ground(&self) -> (Arc<dyn bake::Ground>, [u8; 32]) {
        use sha2::Digest;
        match &self.ground {
            AmbientGround::Elevation(g) => (Arc::new(bake::ElevationGround(g.clone())), elevation_digest(g)),
            AmbientGround::Flat(y) => (Arc::new(bake::FlatGround(*y)), sha2::Sha256::digest(y.to_le_bytes()).into()),
        }
    }
}

/// Two tile indices in one float (exact: under 2^24), for `Look.lx[10]`.
pub fn pack(t: (i32, i32)) -> f32 {
    (((t.0 + 512).clamp(0, 1023) * 1024 + (t.1 + 512).clamp(0, 1023)) as u32) as f32
}

/// A level's resident rectangle of tiles (min inclusive, max exclusive) as two packed floats; (0, 0) for none.
fn rect_row(r: Option<((i32, i32), (i32, i32))>) -> [f32; 2] {
    match r {
        Some((a, b)) if a.0 < b.0 && a.1 < b.1 => [pack(a), pack(b)],
        _ => [0.0, 0.0],
    }
}

/// Texels a side of the volume (every level: 8 tiles of 32 at 4 m, 4 of 64
/// at 2 m) and of the citywide plane (18.4 km at 8 m: the city's buildings and
/// the reach round them at the 1.25 ground plan are 17.2 x 17.9 km). Constants
/// of the shader (ambient.wgsl: no size query, no size in the uniform).
pub const VOLUME_TEXELS: usize = 256;
pub const PLANE_TEXELS: usize = 2304;

/// How much of the volume a pipeline's shader reads (ambient.wgsl
/// `AMB_SHADER_LEVELS`, `AMB_SHADER_ON`). On Apple's GPU compiler the
/// fragment shaders that discard stand at the edge of what compiles: a
/// texture size query, a mapping from sizes in the uniform, or one level of
/// the volume more made `prop-cutout` and `hlod-draw` fail with
/// XPC_ERROR_CONNECTION_INTERRUPTED (measured, stage B3), so they read less.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderLevels {
    /// The plane and both levels, and the ground bounce the levels hold (the
    /// city and its probe face, the structures' concrete, agents, landmark
    /// wires, the CC0 decals).
    Full,
    /// The citywide plane alone: the far layer (its cells lie beyond the
    /// levels) and props (their cut-out pipelines took nothing more). No
    /// ground bounce.
    Plane,
    /// None: the frame without the volume (for a pipeline that cannot take the plane either; none today).
    Off,
}

/// ambient.wgsl and its constants (composed into the city shader), reading what `levels` says.
pub fn wgsl(levels: ShaderLevels) -> String {
    let f = |v: f64| format!("{v:.6}");
    let mut s = String::from(include_str!("ambient.wgsl"));
    s.push_str("\n// generated by ambient.rs wgsl()\n");
    for (n, v) in [
        ("AMB_TILE_M", f(bake::TILE_M)),
        ("AMB_FAR_CELL_M", f(4.0)),
        ("AMB_NEAR_CELL_M", f(2.0)),
        ("AMB_PLANE_CELL_M", f(bake::PLANE_CELL_M)),
        ("AMB_FINE_M", f(bake::FINE_LAYER_M)),
        ("AMB_COARSE_M", f(bake::COARSE_LAYER_M)),
        ("AMB_TOP_C", f(bake::layer_centre(bake::LAYERS - 1))),
        ("AMB_EDGE_FADE_M", f(EDGE_FADE_M)),
        ("AMB_NORMAL_OFFSET_M", f(NORMAL_OFFSET_M)),
        ("AMB_OPEN_RISE_M", f(OPEN_RISE_M)),
        ("AMB_OCCLUDER", f(OCCLUDER_OVER_GROUND)),
        ("AMB_LOBE_SKY_ALBEDO", f(eo_look::bounce::LOBE_SKY_ALBEDO)),
        ("AMB_SUN_SHIFT_COT_MAX", f(SUN_SHIFT_COT_MAX)),
        ("AMB_SUN_SHIFT_MAX_M", f(SUN_SHIFT_MAX_M)),
    ] {
        s.push_str(&format!("const {n}: f32 = {v};\n"));
    }
    let (n, on) = match levels {
        ShaderLevels::Full => (2, true),
        ShaderLevels::Plane => (0, true),
        ShaderLevels::Off => (0, false),
    };
    // (`--occluder-light`) whether the street's own sky is read apart (ambient.wgsl
    // `amb_sample_ground`): where the levels are; the agents' module turns it off,
    // and turns on the stand-in set first thing in the fragment (`AMB_OL_PRE`,
    // `amb_ol_pre`; agents.rs `agents_city_source`).
    let street = levels == ShaderLevels::Full;
    s.push_str(&format!(
        "const AMB_LAYERS: u32 = {}u;\nconst AMB_FINE_LAYERS: u32 = {}u;\nconst AMB_VOLUME_TEXELS: u32 = {VOLUME_TEXELS}u;\nconst AMB_PLANE_TEXELS: u32 = {PLANE_TEXELS}u;\nconst AMB_SHADER_LEVELS: u32 = {n}u;\nconst AMB_SHADER_ON: bool = {on};\nconst AMB_OL_STREET: bool = {street};\nconst AMB_OL_PRE: bool = false;\n",
        bake::LAYERS,
        bake::FINE_LAYERS
    ));
    s
}

/// A bake request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Req {
    Tile { level: u8, at: (i32, i32) },
    Plane { at: (i32, i32) },
}

/// A finished bake: the request, its payload (a plane block's with its texel box), whether from the cache, the bake's time.
struct Done {
    req: Req,
    payload: Vec<u8>,
    plane_box: (u32, u32, u32, u32),
    cached: bool,
    ms: f64,
}

/// What the workers share: the scene (once the setup thread has made it), the requests, the camera.
struct Shared {
    ctx: Mutex<Option<Arc<BakeCtx>>>,
    pending: Mutex<Vec<Req>>,
    cam: Mutex<[f64; 2]>,
    stop: AtomicBool,
    in_flight: AtomicUsize,
}

struct BakeCtx {
    scene: bake::Scene,
    levels: Vec<Level>,
    /// each level's spec (with the open ground's share while the bounce is on).
    specs: Vec<Spec>,
    keys: Vec<String>,
    plane_spec: Spec,
    plane_key: String,
    cache_dir: Option<PathBuf>,
    map: PlaneMap,
}

impl BakeCtx {
    fn rank(&self, r: &Req, cam: [f64; 2]) -> f64 {
        let (c, extra) = match r {
            Req::Tile { at, .. } => ([(at.0 as f64 + 0.5) * bake::TILE_M, (at.1 as f64 + 0.5) * bake::TILE_M], 0.0),
            Req::Plane { at } => {
                let o = self.map.block_origin(*at);
                let s = bake::PLANE_TILE_CELLS as f64 * bake::PLANE_CELL_M;
                ([o[0] + 0.5 * s, o[1] + 0.5 * s], PLANE_RANK_M)
            }
        };
        (c[0] - cam[0]).hypot(c[1] - cam[1]) + extra
    }

    /// From the cache, or baked (and cached).
    fn load_or_bake(&self, r: Req) -> Done {
        let t = Instant::now();
        match r {
            Req::Tile { level, at } => {
                let (lv, spec, key) = (self.levels[level as usize], &self.specs[level as usize], &self.keys[level as usize]);
                // layers in the payload: the sky's, and the ground's after them.
                let parts = if spec.ground { 2 } else { 1 };
                let (c, l) = (lv.cells() as u32, (bake::LAYERS * parts) as u32);
                let x0 = at.0 as f64 * bake::TILE_M;
                let z0 = at.1 as f64 * bake::TILE_M;
                if self.map.misses([x0, z0, x0 + bake::TILE_M, z0 + bake::TILE_M]) {
                    // off the city's plane: open sky (and ground), nothing to bake or keep.
                    return Done { req: r, payload: bake::volume_payload(&Block::open(c as usize, bake::LAYERS, spec.ground)), plane_box: (0, 0, 0, 0), cached: true, ms: 0.0 };
                }
                let path = self.cache_dir.as_ref().map(|d| bake::cache_path(d, key, at));
                if let Some(p) = &path {
                    if let Some(payload) = std::fs::read(p).ok().and_then(|b| bake::decode_cache(&b, at, c, l)).filter(|v| v.len() == (c * c * l * 8) as usize) {
                        return Done { req: r, payload, plane_box: (0, 0, 0, 0), cached: true, ms: t.elapsed().as_secs_f64() * 1e3 };
                    }
                }
                let payload = bake::volume_payload(&bake::bake(&self.scene, spec, x0, z0));
                if let Some(p) = &path {
                    bake::write_atomic(p, &bake::encode_cache(at, c, l, &payload));
                }
                Done { req: r, payload, plane_box: (0, 0, 0, 0), cached: false, ms: t.elapsed().as_secs_f64() * 1e3 }
            }
            Req::Plane { at } => {
                let m = self.map;
                let (bx, bz) = (at.0 as usize * bake::PLANE_TILE_CELLS, at.1 as usize * bake::PLANE_TILE_CELLS);
                let bx_box = (bx as u32, bz as u32, bake::PLANE_TILE_CELLS.min(m.w - bx) as u32, bake::PLANE_TILE_CELLS.min(m.h - bz) as u32);
                let path = self.cache_dir.as_ref().map(|d| bake::cache_path(d, &self.plane_key, at));
                if let Some(p) = &path {
                    if let Some(payload) = std::fs::read(p).ok().and_then(|b| bake::decode_cache(&b, at, bake::PLANE_TILE_CELLS as u32, 4)).filter(|v| v.len() == (bx_box.2 * bx_box.3 * 8) as usize) {
                        return Done { req: r, payload, plane_box: bx_box, cached: true, ms: t.elapsed().as_secs_f64() * 1e3 };
                    }
                }
                let o = m.block_origin(at);
                let block = bake::bake(&self.scene, &self.plane_spec, o[0], o[1]);
                let (bx_box, payload) = bake::plane_payload(&block, &self.scene.ground_ref, at);
                if let Some(p) = &path {
                    bake::write_atomic(p, &bake::encode_cache(at, bake::PLANE_TILE_CELLS as u32, 4, &payload));
                }
                Done { req: r, payload, plane_box: bx_box, cached: false, ms: t.elapsed().as_secs_f64() * 1e3 }
            }
        }
    }
}


/// The elevation grid's digest (its frame, both planes, its height scale).
fn elevation_digest(g: &eo_data::ElevationGrid) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for v in [g.meta.origin_x, g.meta.origin_z, g.meta.cell_size, g.height_scale] {
        h.update(v.to_le_bytes());
    }
    h.update((g.meta.width as u64).to_le_bytes());
    h.update((g.meta.height as u64).to_le_bytes());
    h.update(bytemuck::cast_slice::<f32, u8>(&g.lidar));
    h.update(bytemuck::cast_slice::<i16, u8>(&g.topobathy));
    h.finalize().into()
}

/// A recorded copy from the staging buffer into a box of the volume or the plane.
#[derive(Clone, Copy, Debug)]
struct Copy {
    offset: u64,
    volume: bool,
    origin: [u32; 3],
    size: [u32; 3],
}

/// This frame's writes into the staging buffer, waiting for `encode`.
#[derive(Default)]
struct Copies {
    cursor: u64,
    list: Vec<Copy>,
}

/// A tile's state in its level's ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileState {
    Requested,
    Uploading,
    Resident,
}

/// Bakes and uploads so far (the hand-off's numbers; the bench prints them).
#[derive(Clone, Copy, Debug, Default)]
pub struct AmbientStats {
    /// the setup thread's time (occluders, the plane's ground, the keys).
    pub setup_ms: f64,
    pub tiles_baked: u32,
    pub tiles_cached: u32,
    /// total and largest bake time of the tiles baked (not cached).
    pub tile_ms: f64,
    pub tile_ms_max: f64,
    pub plane_blocks: u32,
    pub plane_baked: u32,
    pub plane_cached: u32,
    pub plane_ms: f64,
    pub plane_blocks_done: u32,
    pub tiles_resident: u32,
    pub uploaded_bytes: u64,
    pub volume_bytes: u64,
    pub plane_bytes: u64,
}

struct State {
    uploads: UploadQueue,
    rx: Receiver<Done>,
    frozen: bool,
    /// each level's ring: its middle corner (a tile index) and its tiles.
    centre: Vec<Option<(i32, i32)>>,
    tiles: Vec<HashMap<(i32, i32), TileState>>,
    /// the plane's open fill queued, its bands still to land, and landed.
    plane_queued: bool,
    plane_bands_left: usize,
    plane_ready: bool,
    stats: AmbientStats,
    said: bool,
}

struct Res {
    levels: Vec<Level>,
    /// the levels hold the open ground's share too (the second half of the volume's depth).
    ground: bool,
    volume: wgpu::Texture,
    plane: wgpu::Texture,
    staging: wgpu::Buffer,
    map: PlaneMap,
    shared: Arc<Shared>,
    /// the plane's open fill (from the setup thread) and the setup's time.
    initial: Arc<Mutex<Option<Vec<u8>>>>,
    setup_ms: Arc<Mutex<Option<f64>>>,
    state: Mutex<State>,
    copies: Mutex<Copies>,
    bytes: u64,
}

pub struct AmbientGpu {
    res: OnceLock<Res>,
}

/// Upload job tags: a plane band, a plane block, or a tile (level, x, z).
const TAG_PLANE_BAND: u64 = 1 << 62;
const TAG_PLANE_BLOCK: u64 = 1 << 61;

fn tile_tag(level: usize, at: (i32, i32)) -> u64 {
    ((level as u64) << 40) | ((((at.0 + 32768) as u64) & 0xffff) << 16) | (((at.1 + 32768) as u64) & 0xffff)
}

fn tag_tile(tag: u64) -> (usize, (i32, i32)) {
    (((tag >> 40) & 0xff) as usize, ((((tag >> 16) & 0xffff) as i64 - 32768) as i32, ((tag & 0xffff) as i64 - 32768) as i32))
}

impl Drop for AmbientGpu {
    fn drop(&mut self) {
        if let Some(r) = self.res.get() {
            r.shared.stop.store(true, Ordering::Relaxed);
        }
    }
}

impl AmbientGpu {
    pub fn new(_device: &wgpu::Device) -> Self {
        Self { res: OnceLock::new() }
    }

    /// Whether `attach` made the volume (with `ambient_volume` on and a city).
    pub fn attached(&self) -> bool {
        self.res.get().is_some()
    }

    /// GPU bytes of the volume, the plane and the staging buffer (0 unattached).
    pub fn bytes(&self) -> u64 {
        self.res.get().map(|r| r.bytes).unwrap_or(0)
    }

    /// Bakes and uploads so far.
    pub fn stats(&self) -> AmbientStats {
        self.res.get().map(|r| r.state.lock().unwrap().stats).unwrap_or_default()
    }

    /// Hold the volume as it is (no re-centring, no new upload): the
    /// occlusion check's frozen frames (`--occlusion-check`), as the probe's.
    pub fn set_frozen(&self, on: bool) {
        if let Some(r) = self.res.get() {
            r.state.lock().unwrap().frozen = on;
        }
    }

    /// Everything wanted round the camera is resident: the plane (its open
    /// fill and every baked block) and every level's whole ring; true when not attached.
    pub fn settled(&self) -> bool {
        let Some(r) = self.res.get() else { return true };
        let st = r.state.lock().unwrap();
        st.plane_ready
            && st.stats.plane_blocks_done >= st.stats.plane_blocks
            && st.uploads.pending() == 0
            && st.centre.iter().zip(&st.tiles).zip(&r.levels).all(|((c, t), l)| c.is_some() && t.len() == l.ring * l.ring && t.values().all(|s| *s == TileState::Resident))
    }

    /// What `settled` waits on, in words (a timed-out settle prints it).
    pub fn unsettled(&self) -> String {
        let Some(r) = self.res.get() else { return String::new() };
        let st = r.state.lock().unwrap();
        let resident: Vec<String> = st.tiles.iter().zip(&r.levels).map(|(t, l)| format!("{} of {}", t.values().filter(|s| **s == TileState::Resident).count(), l.ring * l.ring)).collect();
        format!("ambient: plane {} ({} of {} blocks), tiles resident {:?}, {} uploads pending", if st.plane_ready { "filled" } else { "not filled" }, st.stats.plane_blocks_done, st.stats.plane_blocks, resident, st.uploads.pending())
    }

    /// Make the volume and the plane for the look's `ambient_volume`, bind
    /// them (`LookGpu::set_ambient_volume`) and start the bake: a setup thread
    /// builds the occluders, the plane's ground and the cache keys, then
    /// `inputs.threads` workers bake tiles nearest the camera first. Returns
    /// the GPU bytes allocated (0 with the option off). Call once, at load.
    pub fn attach(&self, ctx: &crate::GpuContext, look: &LookGpu, inputs: AmbientInputs, camera: [f64; 2]) -> u64 {
        let levels = levels(look.options.lighting.ambient_volume);
        if levels.is_empty() || self.res.get().is_some() {
            return 0;
        }
        // the ground bounce's open ground in the levels (the volume's second half).
        let open_ground = look.options.lighting.ground_bounce_on();
        let parts = if open_ground { 2 } else { 1 };
        let t0 = Instant::now();
        // the plane over the buildings' bounds (and the reach round them).
        let bounds = inputs.bounds().unwrap_or([-bake::TILE_M, -bake::TILE_M, bake::TILE_M, bake::TILE_M]);
        let map = PlaneMap::centred(bounds, PLANE_TEXELS);
        if !map.holds(bounds) {
            eprintln!("ambient: the buildings ({:.0} x {:.0} m and the reach) are wider than the plane's {:.0} m: past its edge the sky counts as open", bounds[2] - bounds[0], bounds[3] - bounds[1], PLANE_TEXELS as f64 * bake::PLANE_CELL_M);
        }
        let d = &ctx.device;
        let n = levels[0].texels() as u32;
        let depth = (levels.len() * bake::LAYERS * parts) as u32;
        let tex = |label: &str, size: wgpu::Extent3d, dimension: wgpu::TextureDimension| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let volume = tex("ambient-volume", wgpu::Extent3d { width: n, height: n, depth_or_array_layers: depth }, wgpu::TextureDimension::D3);
        let plane = tex("ambient-plane", wgpu::Extent3d { width: map.w as u32, height: map.h as u32, depth_or_array_layers: 1 }, wgpu::TextureDimension::D2);
        let staging = d.create_buffer(&wgpu::BufferDescriptor { label: Some("ambient-staging"), size: STAGING_BYTES, usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let volume_bytes = n as u64 * n as u64 * depth as u64 * 8;
        let plane_bytes = (map.w * map.h * 8) as u64;
        look.set_ambient_volume(Some(&volume.create_view(&Default::default())), Some(&plane.create_view(&Default::default())));
        let shared = Arc::new(Shared { ctx: Mutex::new(None), pending: Mutex::new(vec![]), cam: Mutex::new(camera), stop: AtomicBool::new(false), in_flight: AtomicUsize::new(0) });
        let (tx, rx): (Sender<Done>, Receiver<Done>) = std::sync::mpsc::channel();
        let initial: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let setup_ms: Arc<Mutex<Option<f64>>> = Arc::new(Mutex::new(None));
        let workers = inputs.threads.max(1);
        // the setup thread: occluders, the plane's ground, the keys; then the workers start.
        {
            let (shared, levels, initial, setup_ms) = (shared.clone(), levels.clone(), initial.clone(), setup_ms.clone());
            let threads = inputs.threads.max(1);
            std::thread::Builder::new()
                .name("eo-ambient-setup".into())
                .spawn(move || {
                    crate::hitch::worker_started();
                    let t = Instant::now();
                    let (ground, ground_digest) = inputs.ground();
                    let prisms: Vec<bake::Prism> = inputs.rings().into_iter().filter_map(|(ring, h)| bake::Prism::standing(ring, h, inputs.lift, ground.as_ref())).collect();
                    let mut decks = inputs.structures.as_deref().map(bake::deck_pieces).unwrap_or_default();
                    decks.extend_from_slice(&inputs.decks);
                    let ground_ref = Arc::new(GroundRef::sample(map, ground.as_ref(), threads));
                    let scene = bake::Scene::new(prisms, decks, ground, ground_ref.clone());
                    // the scene key: the occluders, the ground, the ground plan's scales, the structures (when they add decks).
                    use sha2::Digest;
                    let mut h = sha2::Sha256::new();
                    h.update(scene.occluder_digest());
                    h.update(ground_digest);
                    h.update(eo_data::groundscale::scale().to_le_bytes());
                    h.update(eo_data::groundscale::height_scale().to_le_bytes());
                    h.update(inputs.lift.to_le_bytes());
                    if !scene.decks.is_empty() {
                        h.update(b"structures");
                        h.update(scene.deck_digest());
                    }
                    for v in [map.min[0], map.min[1], map.w as f64, map.h as f64] {
                        h.update(v.to_le_bytes());
                    }
                    let scene_key: [u8; 32] = h.finalize().into();
                    let specs: Vec<Spec> = levels.iter().map(|l| Spec::volume(l.cell).with_ground(open_ground)).collect();
                    let keys: Vec<String> = specs.iter().map(|s| bake::level_key(&scene_key, s)).collect();
                    let plane_spec = Spec::plane();
                    let plane_key = bake::level_key(&scene_key, &plane_spec);
                    *initial.lock().unwrap() = Some(bake::plane_initial(&ground_ref));
                    shared.pending.lock().unwrap().extend(map.blocks().into_iter().map(|at| Req::Plane { at }));
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    eprintln!(
                        "ambient: setup {ms:.0} ms: {} prisms, {} deck pieces, plane {} x {} cells of {} m from ({:.0}, {:.0}), {} blocks; keys {} (plane {plane_key}); cache {}",
                        scene.prisms.len(),
                        scene.decks.len(),
                        map.w,
                        map.h,
                        bake::PLANE_CELL_M,
                        map.min[0],
                        map.min[1],
                        map.blocks().len(),
                        keys.join(", "),
                        match &inputs.cache_dir {
                            Some(d) => format!("{}", d.join("ambient").display()),
                            None => "off".into(),
                        }
                    );
                    *setup_ms.lock().unwrap() = Some(ms);
                    *shared.ctx.lock().unwrap() = Some(Arc::new(BakeCtx { scene, levels, specs, keys, plane_spec, plane_key, cache_dir: inputs.cache_dir, map }));
                })
                .expect("ambient setup thread");
        }
        for k in 0..workers {
            let (shared, tx) = (shared.clone(), tx.clone());
            std::thread::Builder::new()
                .name(format!("eo-ambient-{k}"))
                .spawn(move || {
                    crate::hitch::worker_started(); // below the render thread
                    while !shared.stop.load(Ordering::Relaxed) {
                        let Some(ctx) = shared.ctx.lock().unwrap().clone() else {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        };
                        let next = {
                            let cam = *shared.cam.lock().unwrap();
                            let mut p = shared.pending.lock().unwrap();
                            let best = p.iter().enumerate().map(|(k, r)| (k, ctx.rank(r, cam))).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(k, _)| k);
                            best.map(|k| {
                                shared.in_flight.fetch_add(1, Ordering::SeqCst);
                                p.swap_remove(k)
                            })
                        };
                        let Some(req) = next else {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            continue;
                        };
                        let done = ctx.load_or_bake(req);
                        shared.in_flight.fetch_sub(1, Ordering::SeqCst);
                        if tx.send(done).is_err() {
                            break;
                        }
                    }
                })
                .expect("ambient worker thread");
        }
        let nl = levels.len();
        let state = State {
            uploads: UploadQueue::new(UPLOAD_BUDGET).with_ring(false),
            rx,
            frozen: false,
            centre: vec![None; nl],
            tiles: vec![HashMap::new(); nl],
            plane_queued: false,
            plane_bands_left: 0,
            plane_ready: false,
            stats: AmbientStats { volume_bytes, plane_bytes, plane_blocks: map.blocks().len() as u32, ..Default::default() },
            said: false,
        };
        let bytes = volume_bytes + plane_bytes + STAGING_BYTES;
        eprintln!(
            "ambient: {nl} level(s) (cell m, ring tiles, reach m) {:?}{}; volume {n} x {n} x {depth} ({:.1} MB), plane {} x {} ({:.1} MB), staging {:.1} MB; made in {:.1} ms",
            levels.iter().map(|l| (l.cell, l.ring, l.reach_m().round())).collect::<Vec<_>>(),
            if open_ground { " with the ground bounce's open ground" } else { "" },
            volume_bytes as f64 / 1e6,
            map.w,
            map.h,
            plane_bytes as f64 / 1e6,
            STAGING_BYTES as f64 / 1e6,
            t0.elapsed().as_secs_f64() * 1e3
        );
        let _ = self.res.set(Res { levels, ground: open_ground, volume, plane, staging, map, shared, initial, setup_ms, state: Mutex::new(state), copies: Mutex::new(Copies::default()), bytes });
        bytes
    }

    /// The CPU side, every frame in `Renderer::update` before the look's
    /// uniform is written (the rows set here land in this frame): copies a
    /// frame without an `encode` left are submitted; finished bakes become
    /// upload jobs; the rings re-centre on the camera; the queue writes this
    /// frame's budget into the staging buffer; the resident rectangles go to
    /// `Look.lx[10..11]`.
    pub fn update(&self, renderer: &crate::Renderer, queue: &wgpu::Queue, cam: &Camera) {
        let Some(r) = self.res.get() else { return };
        if renderer.look.options.lighting.ambient_volume == 0 {
            renderer.look.set_lighting_params(LX_AMBIENT, &[[0.0; 4]; 2]);
            renderer.look.set_lighting_params(LX_FLAGS, &[[0.0; 4]]);
            return;
        }
        // the ground bounce's sun exitance (row 0 y z w; zero while off, and with the sun down).
        let bounce = if r.ground && renderer.look.options.lighting.ground_bounce_on() { eo_look::bounce::sun_exitance(&renderer.look.frame()) } else { [0.0; 3] };
        renderer.look.set_lighting_params(LX_FLAGS, &[[0.0, bounce[0] as f32, bounce[1] as f32, bounce[2] as f32]]);
        self.flush(renderer, queue);
        let mut st = r.state.lock().unwrap();
        if st.frozen {
            return;
        }
        let eye = [cam.eye.x as f64, cam.eye.z as f64];
        crate::hitch::try_set(&r.shared.cam, eye);
        if let Some(ms) = r.setup_ms.lock().unwrap().take() {
            st.stats.setup_ms = ms;
        }
        // the plane's open fill first (every later write lands over it).
        if !st.plane_queued {
            if let Some(bytes) = r.initial.lock().unwrap().take() {
                // in bands of whole rows, one a frame (the budget's worth).
                let row = r.map.w * 8;
                let band_rows = (UPLOAD_BUDGET as usize / row).max(1);
                let mut left = 0;
                for (k, band) in bytes.chunks(row * band_rows).enumerate() {
                    let rows = band.len() / row;
                    let target = Target::AmbientPlane { x: 0, z: (k * band_rows) as u16, w: r.map.w as u16, h: rows as u16 };
                    st.uploads.push(Job::new(TAG_PLANE_BAND | k as u64, vec![Write { target, offset: 0, data: Data::Bytes(band.to_vec()) }], vec![]));
                    left += 1;
                }
                st.plane_queued = true;
                st.plane_bands_left = left;
            }
        }
        // finished bakes.
        while let Ok(done) = st.rx.try_recv() {
            match done.req {
                Req::Tile { level, at } => {
                    if done.cached {
                        st.stats.tiles_cached += 1;
                    } else {
                        st.stats.tiles_baked += 1;
                        st.stats.tile_ms += done.ms;
                        st.stats.tile_ms_max = st.stats.tile_ms_max.max(done.ms);
                    }
                    let l = level as usize;
                    if st.tiles[l].get(&at) != Some(&TileState::Requested) {
                        continue; // left the ring while it baked
                    }
                    st.tiles[l].insert(at, TileState::Uploading);
                    let target = Target::AmbientVolume { level, x: at.0 as i16, z: at.1 as i16 };
                    let mut sky = done.payload;
                    // with the ground: two writes (the sky's half, then the ground's at its byte offset),
                    // so a frame's upload stays within the budget (a 2 m tile's halves are 768 KiB each).
                    let writes = if r.ground {
                        let half = sky.len() / 2;
                        let g = sky.split_off(half);
                        vec![Write { target, offset: 0, data: Data::Bytes(sky) }, Write { target, offset: half as u64, data: Data::Bytes(g) }]
                    } else {
                        vec![Write { target, offset: 0, data: Data::Bytes(sky) }]
                    };
                    st.uploads.push(Job::new(tile_tag(l, at), writes, vec![]));
                }
                Req::Plane { at } => {
                    if done.cached {
                        st.stats.plane_cached += 1;
                    } else {
                        st.stats.plane_baked += 1;
                        st.stats.plane_ms += done.ms;
                    }
                    let (x, z, w, h) = done.plane_box;
                    if w == 0 || h == 0 {
                        st.stats.plane_blocks_done += 1;
                        continue;
                    }
                    let target = Target::AmbientPlane { x: x as u16, z: z as u16, w: w as u16, h: h as u16 };
                    let tag = TAG_PLANE_BLOCK | (((at.0 as u64) & 0xffff) << 16) | ((at.1 as u64) & 0xffff);
                    st.uploads.push(Job::new(tag, vec![Write { target, offset: 0, data: Data::Bytes(done.payload) }], vec![]));
                }
            }
        }
        // the rings follow the camera (once the bake can take requests: the plane's ground is known).
        if r.shared.ctx.lock().unwrap().is_some() {
            for l in 0..r.levels.len() {
                self.recentre(r, &mut st, l, eye);
            }
        }
        let (pump, done) = st.uploads.pump(renderer, queue);
        st.stats.uploaded_bytes += pump.bytes;
        for tag in done {
            if tag & TAG_PLANE_BAND != 0 {
                st.plane_bands_left = st.plane_bands_left.saturating_sub(1);
                if st.plane_bands_left == 0 {
                    st.plane_ready = true;
                }
            } else if tag & TAG_PLANE_BLOCK != 0 {
                st.stats.plane_blocks_done += 1;
            } else {
                let (l, at) = tag_tile(tag);
                if let Some(s) = st.tiles[l].get_mut(&at) {
                    *s = TileState::Resident;
                }
            }
        }
        st.stats.tiles_resident = st.tiles.iter().map(|t| t.values().filter(|s| **s == TileState::Resident).count() as u32).sum();
        // the rows: each level's resident rectangle, the plane's corner (inert until its fill has landed).
        let rect = |l: usize| -> [f32; 2] {
            if !st.plane_ready || l >= r.levels.len() {
                return [0.0, 0.0];
            }
            rect_row(resident_rect(&st, r.levels[l], l, eye))
        };
        let (far, near) = (rect(0), rect(1));
        // the plane's corner and cell (0: not landed, the feature inert), 1 / the volume's depth in layers.
        let plane_cell = if st.plane_ready { bake::PLANE_CELL_M as f32 } else { 0.0 };
        let inv_depth = 1.0 / (r.levels.len() * bake::LAYERS * if r.ground { 2 } else { 1 }) as f32;
        renderer.look.set_lighting_params(LX_AMBIENT, &[[far[0], far[1], near[0], near[1]], [r.map.min[0] as f32, r.map.min[1] as f32, plane_cell, inv_depth]]);
        if !st.said && st.plane_ready && st.stats.plane_blocks_done >= st.stats.plane_blocks && st.tiles.iter().zip(&r.levels).all(|(t, l)| t.len() == l.ring * l.ring && t.values().all(|s| *s == TileState::Resident)) {
            st.said = true;
            let s = st.stats;
            eprintln!(
                "ambient: settled: setup {:.0} ms; plane {} blocks ({} baked, {:.1} s in all, {:.0} ms a block; {} cached); tiles {} resident ({} baked, {:.1} ms a tile, at most {:.1} ms; {} cached); {:.1} MB uploaded; GPU {:.1} MB volume + {:.1} MB plane",
                s.setup_ms,
                s.plane_blocks,
                s.plane_baked,
                s.plane_ms / 1e3,
                s.plane_ms / s.plane_baked.max(1) as f64,
                s.plane_cached,
                s.tiles_resident,
                s.tiles_baked,
                s.tile_ms / s.tiles_baked.max(1) as f64,
                s.tile_ms_max,
                s.tiles_cached,
                s.uploaded_bytes as f64 / 1e6,
                s.volume_bytes as f64 / 1e6,
                s.plane_bytes as f64 / 1e6
            );
        }
    }

    /// Re-centre level `l`'s ring on the camera: drop the tiles leaving it
    /// (their queued uploads and requests too), ask for the ones entering.
    fn recentre(&self, r: &Res, st: &mut State, l: usize, eye: [f64; 2]) {
        let lv = r.levels[l];
        let t = bake::TILE_M;
        let corner = ((eye[0] / t).round() as i32, (eye[1] / t).round() as i32);
        let keep = match st.centre[l] {
            Some(c) => {
                let (cx, cz) = (c.0 as f64 * t, c.1 as f64 * t);
                (eye[0] - cx).abs() <= 0.5 * t + RECENTRE_SLACK_M && (eye[1] - cz).abs() <= 0.5 * t + RECENTRE_SLACK_M
            }
            None => false,
        };
        if keep {
            return;
        }
        st.centre[l] = Some(corner);
        let half = (lv.ring / 2) as i32;
        let (x0, z0) = (corner.0 - half, corner.1 - half);
        let inside = |at: &(i32, i32)| at.0 >= x0 && at.0 < x0 + lv.ring as i32 && at.1 >= z0 && at.1 < z0 + lv.ring as i32;
        let gone: Vec<(i32, i32)> = st.tiles[l].keys().filter(|at| !inside(at)).copied().collect();
        if !gone.is_empty() {
            for at in &gone {
                st.tiles[l].remove(at);
            }
            let tags: std::collections::HashSet<u64> = gone.iter().map(|at| tile_tag(l, *at)).collect();
            st.uploads.cancel(|tag| !tags.contains(&tag));
            r.shared.pending.lock().unwrap().retain(|q| !matches!(q, Req::Tile { level, at } if *level as usize == l && gone.contains(at)));
        }
        let mut asks = vec![];
        for z in z0..z0 + lv.ring as i32 {
            for x in x0..x0 + lv.ring as i32 {
                if !st.tiles[l].contains_key(&(x, z)) {
                    st.tiles[l].insert((x, z), TileState::Requested);
                    asks.push(Req::Tile { level: l as u8, at: (x, z) });
                }
            }
        }
        r.shared.pending.lock().unwrap().extend(asks);
    }

    /// Submit the copies a frame wrote without encoding them (a settle loop's `update` alone).
    fn flush(&self, renderer: &crate::Renderer, queue: &wgpu::Queue) {
        let Some(r) = self.res.get() else { return };
        let list = {
            let mut c = r.copies.lock().unwrap();
            c.cursor = 0;
            std::mem::take(&mut c.list)
        };
        if list.is_empty() {
            return;
        }
        let mut enc = renderer.gpu_device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ambient-flush") });
        Self::record(r, &mut enc, &list);
        queue.submit([enc.finish()]);
    }

    fn record(r: &Res, enc: &mut wgpu::CommandEncoder, list: &[Copy]) {
        for c in list {
            let (tex, row) = if c.volume { (&r.volume, c.size[0] * 8) } else { (&r.plane, c.size[0] * 8) };
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo { buffer: &r.staging, layout: wgpu::TexelCopyBufferLayout { offset: c.offset, bytes_per_row: Some(row), rows_per_image: Some(c.size[1]) } },
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: c.origin[0], y: c.origin[1], z: c.origin[2] }, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: c.size[0], height: c.size[1], depth_or_array_layers: c.size[2] },
            );
        }
    }

    /// One upload (`Renderer::write` for `Target::AmbientVolume` and
    /// `Target::AmbientPlane`): the bytes go into the staging buffer and the
    /// copy into the texture is recorded for `encode`. Rows of 256 bytes or
    /// more (a 4 m tile's 32 texels, the plane's whole rows); a write that does
    /// not fit the staging buffer, or whose rows are not 256-byte multiples,
    /// is written straight into the texture. A tile comes as its sky part
    /// (`offset` 0) and, with the ground bounce, its ground part (`offset` the
    /// sky part's bytes), each a whole box of layers.
    pub fn write(&self, queue: &wgpu::Queue, target: Target, _offset: u64, data: &[u8]) {
        let Some(r) = self.res.get() else { return };
        let (volume, origin, size) = match target {
            Target::AmbientVolume { level, x, z } => {
                let Some(lv) = r.levels.get(level as usize) else { return };
                let c = lv.cells() as u32;
                let ring = lv.ring as i32;
                let (sx, sz) = ((x as i32).rem_euclid(ring) as u32 * c, (z as i32).rem_euclid(ring) as u32 * c);
                // the sky's part at offset 0, the ground's (the tile's second half) in the volume's second half.
                let part_bytes = c as u64 * c as u64 * bake::LAYERS as u64 * 8;
                let slab = match _offset {
                    0 => lv.slab,
                    o if r.ground && o == part_bytes => r.levels.len() + lv.slab,
                    _ => return,
                };
                (true, [sx, sz, (slab * bake::LAYERS) as u32], [c, c, bake::LAYERS as u32])
            }
            Target::AmbientPlane { x, z, w, h } => (false, [x as u32, z as u32, 0], [w as u32, h as u32, 1]),
            _ => return,
        };
        let len = data.len() as u64;
        if len != size[0] as u64 * size[1] as u64 * size[2] as u64 * 8 {
            return; // not the box's bytes: never write a wrong shape
        }
        let row = size[0] as u64 * 8;
        let mut c = r.copies.lock().unwrap();
        let at = c.cursor.next_multiple_of(256);
        if row % 256 == 0 && at + len <= STAGING_BYTES {
            queue.write_buffer(&r.staging, at, data);
            c.list.push(Copy { offset: at, volume, origin, size });
            c.cursor = at + len;
        } else {
            let tex = if volume { &r.volume } else { &r.plane };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: origin[0], y: origin[1], z: origin[2] }, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(size[1]) },
                wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
            );
        }
    }

    /// The GPU side of this frame's uploads, before the city pass: the
    /// staging buffer's copies into the volume and the plane, between the
    /// two writes of `lighting::Q_AMBIENT_UPLOAD` (an empty span when nothing
    /// was written, or the feature is off).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu, ts: Ts<'_>) {
        let list = match self.res.get() {
            Some(r) if look.options.lighting.ambient_volume > 0 => {
                let mut c = r.copies.lock().unwrap();
                c.cursor = 0;
                std::mem::take(&mut c.list)
            }
            _ => vec![],
        };
        let Some(r) = self.res.get().filter(|_| !list.is_empty()) else {
            let label = if look.options.lighting.ambient_volume == 0 { "ambient upload (off)" } else { "ambient upload" };
            lighting::empty_span(enc, ts, lighting::Q_AMBIENT_UPLOAD, label);
            return;
        };
        if let Some((q, b)) = ts {
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ambient upload begin"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b + lighting::Q_AMBIENT_UPLOAD), end_of_pass_write_index: None }),
            });
        }
        Self::record(r, enc, &list);
        if let Some((q, b)) = ts {
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ambient upload end"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + lighting::Q_AMBIENT_UPLOAD + 1) }),
            });
        }
    }
}

/// Level `l`'s resident rectangle of tiles inside its ring (min inclusive,
/// max exclusive): the ring, its edge strips peeled while any of their tiles
/// is not resident (the strip with most missing first; on a tie the one
/// furthest from the camera), so it is never a slot being rewritten.
fn resident_rect(st: &State, lv: Level, l: usize, eye: [f64; 2]) -> Option<((i32, i32), (i32, i32))> {
    let c = st.centre[l]?;
    let half = (lv.ring / 2) as i32;
    let (mut x0, mut z0, mut x1, mut z1) = (c.0 - half, c.1 - half, c.0 - half + lv.ring as i32, c.1 - half + lv.ring as i32);
    let ok = |x: i32, z: i32| st.tiles[l].get(&(x, z)) == Some(&TileState::Resident);
    let t = bake::TILE_M;
    loop {
        if x0 >= x1 || z0 >= z1 {
            return None;
        }
        let miss = [(x0..x1).filter(|&x| !ok(x, z0)).count(), (x0..x1).filter(|&x| !ok(x, z1 - 1)).count(), (z0..z1).filter(|&z| !ok(x0, z)).count(), (z0..z1).filter(|&z| !ok(x1 - 1, z)).count()];
        if miss.iter().all(|&m| m == 0) {
            // every edge whole: the inside is too only if nothing is missing within.
            if (x0..x1).all(|x| (z0..z1).all(|z| ok(x, z))) {
                return Some(((x0, z0), (x1, z1)));
            }
            // a hole inside a whole rim: shrink from the side nearer the hole... rare; drop the far edge.
        }
        // distances of the four strips from the eye (to prefer peeling the far one).
        let dist = [(eye[1] - z0 as f64 * t).abs(), (eye[1] - z1 as f64 * t).abs(), (eye[0] - x0 as f64 * t).abs(), (eye[0] - x1 as f64 * t).abs()];
        let k = (0..4).max_by(|&a, &b| miss[a].cmp(&miss[b]).then(dist[a].total_cmp(&dist[b]))).unwrap();
        match k {
            0 => z0 += 1,
            1 => z1 -= 1,
            2 => x0 += 1,
            _ => x1 -= 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_packing_round_trip() {
        for (l, at) in [(0usize, (0, 0)), (1, (-79, 54)), (0, (12, -300)), (1, (32000, -32000))] {
            assert_eq!(tag_tile(tile_tag(l, at)), (l, at));
            assert_eq!(tile_tag(l, at) & (TAG_PLANE_BAND | TAG_PLANE_BLOCK), 0);
        }
        for t in [(0, 0), (-80, 60), (100, -100), (-512, 511)] {
            let p = pack(t) as u32;
            assert_eq!(((p / 1024) as i32 - 512, (p % 1024) as i32 - 512), t);
            assert_eq!(pack(t) as u32 as f32, pack(t), "exact in a float");
        }
    }

    #[test]
    fn the_levels_reach_what_the_tiers_promise() {
        let high = levels(1);
        assert_eq!(high.len(), 1);
        assert!(high[0].reach_m() >= 300.0 && high[0].cell == 4.0, "{:?} reaches {}", high[0], high[0].reach_m());
        let hyper = levels(2);
        assert_eq!(hyper[0].texels(), hyper[1].texels(), "the slabs share one texture's width");
        assert!(high.iter().chain(&hyper).all(|l| l.texels() == VOLUME_TEXELS), "the shader's constant size");
        assert!(hyper[0].reach_m() >= 400.0 && hyper[1].cell == 2.0 && hyper[1].reach_m() >= 150.0);
        // a 256-byte row: whole tiles of 32 (4 m) or 64 (2 m) texels.
        for l in high.iter().chain(&hyper) {
            assert_eq!((l.cells() * 8) % 256, 0);
        }
        assert!(levels(0).is_empty());
    }

    #[test]
    fn the_ground_bounce_takes_the_lobe_as_the_sky_builds_it_and_only_in_full_shaders() {
        // the lobe's sky part is taken at the albedo eo-sky builds the night's lobe with.
        assert_eq!(eo_look::bounce::LOBE_SKY_ALBEDO, eo_sky::frame::NIGHT_GROUND_ALBEDO);
        let full = wgsl(ShaderLevels::Full);
        assert!(full.contains("const AMB_LOBE_SKY_ALBEDO: f32 = 0.180000;"), "the constant reaches the shader");
        // the bounce reads the levels: the plane-only shaders (props, the far layer) compile it out.
        assert!(wgsl(ShaderLevels::Plane).contains("const AMB_SHADER_LEVELS: u32 = 0u;"));
        assert!(full.contains("const AMB_SHADER_LEVELS: u32 = 2u;"));
        let light = include_str!("look_light.wgsl");
        let at = light.find("amb_bounce(").expect("look_light calls amb_bounce");
        let guard = light[..at].rfind("if (AMB_SHADER_LEVELS > 0u)").expect("behind the levels' constant");
        assert!(at - guard < 200, "the bounce sits right behind AMB_SHADER_LEVELS > 0u");
    }
}
