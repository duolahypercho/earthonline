//! Streets in the GPU-driven renderer (slice 4).
//!
//! Every 128 m tile that has street is one LOD group with three levels:
//!
//!   level 0  near pack   the full street surface of the tile (eo-street's
//!                        port of street-surface-v2: cambered carriageway,
//!                        gutters, kerb faces, kerb tops, kerb ramps, footways,
//!                        frontage, verge banks, junction pads, corner returns,
//!                        lane paint, crosswalks, limit lines), cooked on a
//!                        worker and streamed through the budgeted upload queue
//!   level 1  far strips  carriageway and footways as flat strips, no kerb
//!                        face, no paint (static, built at load)
//!   level 2  coarse      one strip a run over every 4th station (static)
//!
//! The LOD pass picks the level from the camera's distance to the group's
//! bounds (`street` radii in `CullParams`) and what is resident; the cull pass
//! draws the objects of that level only, so a tile is never drawn twice or
//! missing. Near packs are two objects: the textured surface (kit palette
//! materials: asphalt, precast concrete, cast stone, per vertex) and the
//! vertex-colour paint and bank.

use crate::authored::GpuLodGroup;
use crate::kit::RangeAlloc;
use crate::scene::{GpuObject, LOD_STREET_COARSE, LOD_STREET_FAR, LOD_STREET_NEAR, MATERIAL_VERTEX_COLOUR, ObjectKind, Vertex};
use crate::upload::{Data, Job, Target, UploadQueue, Write};
use eo_street::{Layer, StreetWorld, TileMesh};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const GROUP_KIND_STREET: u32 = 2;
pub const TAG_STREET: u64 = 1 << 59;

/// Camera-to-bounds radii: near enter / exit (level 0), mid enter / exit (level 1).
pub const DEFAULT_STREET_RADII: [f32; 4] = [220.0, 238.0, 1200.0, 1296.0];

#[derive(Clone, Copy, Debug)]
pub struct StreetRadii {
    /// Camera to the tile's rectangle: request the near pack within, drop past.
    pub near_load: f32,
    pub near_drop: f32,
}

impl Default for StreetRadii {
    fn default() -> Self {
        Self { near_load: 250.0, near_drop: 310.0 }
    }
}

/// the street surface material (`F_STREET`, draw.wgsl
/// `street_surface`): the JS ground materials over the scanned sets, the class
/// of each vertex in its alpha byte.
#[derive(Clone, Copy, Debug)]
pub struct StreetPalette {
    /// Index of the street material in the material table.
    pub base: u32,
}

/// Street classes (the vertex alpha byte; mirrored as SC_* in draw.wgsl).
pub mod class {
    pub const ASPHALT: u8 = 0;
    pub const GUTTER: u8 = 1;
    pub const PAD: u8 = 2;
    pub const KERB_FACE: u8 = 3;
    pub const KERB_TOP: u8 = 4;
    pub const FOOTWAY: u8 = 5;
    pub const RAMP: u8 = 6;
    pub const PAINT: u8 = 7;
    pub const VERGE: u8 = 8;
}

impl StreetPalette {
    pub fn new(material: u32) -> Self {
        Self { base: material }
    }
}

/// The street surface material for the material table.
pub fn street_material() -> crate::authored::GpuMaterial {
    let mut m = crate::authored::GpuMaterial::vertex_colour();
    m.flags = crate::authored::F_STREET;
    m
}

pub struct StreetTile {
    pub key: (i32, i32),
    pub min: [f32; 2],
    pub max: [f32; 2],
}

pub struct StreetSet {
    pub world: Arc<StreetWorld>,
    pub tiles: Vec<StreetTile>,
    pub tile_of_key: HashMap<(i32, i32), u32>,
    pub group_base: u32,
    pub groups: Vec<GpuLodGroup>,
    pub palette: Option<StreetPalette>,
}

/// The street class of a vertex (the JS ground material it belongs to), and
/// whether it is drawn by the street material (else the paint/bank object).
fn vertex_class(layer: Layer, color: u32) -> (u8, bool) {
    let rgb = color & 0x00ff_ffff;
    match layer {
        Layer::Carriageway => {
            (if rgb == eo_street::mesh::gutter_rgb() { class::GUTTER } else if rgb == eo_street::mesh::pad_rgb() { class::PAD } else { class::ASPHALT }, true)
        }
        Layer::CurbFace => (class::KERB_FACE, true),
        Layer::CurbTop => (class::KERB_TOP, true),
        Layer::Ramp => (class::RAMP, true),
        Layer::Sidewalk => (class::FOOTWAY, true),
        Layer::Verge => (class::VERGE, false),
        Layer::Marking => (class::PAINT, false),
    }
}

/// (`--street-pbr`) the carriageway drawn with smooth normals (`smooth_carriageway`).
static SMOOTH_CARRIAGEWAY: AtomicBool = AtomicBool::new(false);

/// eo-app, before any street tile is cooked: whether the carriageway's
/// normals are smoothed (`--street-pbr`; off: each primitive's flat normal, as before).
pub fn set_smooth_carriageway(on: bool) {
    SMOOTH_CARRIAGEWAY.store(on, Ordering::Relaxed);
}

/// The carriageway's normals averaged over its primitives at each shared
/// corner (angle-weighted, positions welded to a centimetre), so the camber's
/// strips and a junction pad's fan shade as the one smooth surface they are:
/// each primitive keeps a flat normal of its own, and under a low sun the
/// facets show as tone wedges converging on the fan's apex. The kerb, walk and
/// paint keep their own (their edges are real). None when there is nothing to do.
pub fn smooth_carriageway(m: &TileMesh) -> Option<Vec<[f32; 3]>> {
    let key = |p: [f32; 3]| ((p[0] as f64 * 100.0).round() as i64, (p[1] as f64 * 100.0).round() as i64, (p[2] as f64 * 100.0).round() as i64);
    let mut sum: HashMap<(i64, i64, i64), [f64; 3]> = HashMap::new();
    let mut any = false;
    for (t, layer) in m.layers.iter().enumerate() {
        if *layer != Layer::Carriageway {
            continue;
        }
        let tri = [m.indices[3 * t] as usize, m.indices[3 * t + 1] as usize, m.indices[3 * t + 2] as usize];
        let p = tri.map(|i| m.positions[i].map(|v| v as f64));
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        if n[1] < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if nl < 1e-12 {
            continue;
        }
        for c in 0..3 {
            // each corner weighted by its angle (a quad's two triangles count as the quad)
            let (a, b) = (p[(c + 1) % 3], p[(c + 2) % 3]);
            let u = [a[0] - p[c][0], a[1] - p[c][1], a[2] - p[c][2]];
            let v = [b[0] - p[c][0], b[1] - p[c][1], b[2] - p[c][2]];
            let lu = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
            let lv = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if lu < 1e-9 || lv < 1e-9 {
                continue;
            }
            let w = ((u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) / (lu * lv)).clamp(-1.0, 1.0).acos();
            let s = sum.entry(key(m.positions[tri[c]])).or_insert([0.0; 3]);
            for k in 0..3 {
                s[k] += n[k] / nl * w;
            }
        }
        any = true;
    }
    if !any {
        return None;
    }
    let mut out = m.normals.clone();
    for (t, layer) in m.layers.iter().enumerate() {
        if *layer != Layer::Carriageway {
            continue;
        }
        for k in 0..3 {
            let i = m.indices[3 * t + k] as usize;
            if let Some(s) = sum.get(&key(m.positions[i])) {
                let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
                if l > 1e-12 {
                    out[i] = [(s[0] / l) as f32, (s[1] / l) as f32, (s[2] / l) as f32];
                }
            }
        }
    }
    Some(out)
}

/// Split a tile mesh into the street-material part and the plain part (paint,
/// banks), as GPU vertices: rgb the JS palette tint, alpha the street class,
/// the normal's w the distance to the kerb line (/ 6.35 m, -1 none).
/// Returns (vertices, indices of the street object, indices of the plain object).
pub fn to_gpu(m: &TileMesh, pal: Option<&StreetPalette>) -> (Vec<Vertex>, Vec<u32>, Vec<u32>) {
    let smooth = if SMOOTH_CARRIAGEWAY.load(Ordering::Relaxed) { smooth_carriageway(m) } else { None };
    let normals = smooth.as_deref().unwrap_or(&m.normals);
    let mut vcls: Vec<(u8, bool)> = vec![(class::PAINT, false); m.positions.len()];
    for (t, layer) in m.layers.iter().enumerate() {
        for k in 0..3 {
            let v = m.indices[3 * t + k] as usize;
            vcls[v] = vertex_class(*layer, m.colors[v]);
        }
    }
    let textured = pal.is_some();
    let vertices: Vec<Vertex> = (0..m.positions.len())
        .map(|i| {
            let c = m.colors[i];
            let color = (c & 0x00ff_ffff) | ((vcls[i].0 as u32) << 24);
            let mut normal = eo_assets::glb::pack_normal(Vec3::from(normals[i]));
            let kerb = m.kerb.get(i).copied().unwrap_or(-1.0);
            let w: i8 = if kerb >= 0.0 { ((kerb / 6.35).min(1.0) * 127.0).round() as i8 } else { -127 };
            normal = (normal & 0x00ff_ffff) | ((w as u8 as u32) << 24);
            Vertex { pos: m.positions[i], normal, color }
        })
        .collect();
    let (mut tex, mut plain) = (vec![], vec![]);
    for (t, _) in m.layers.iter().enumerate() {
        let tri = &m.indices[3 * t..3 * t + 3];
        if textured && vcls[tri[0] as usize].1 { tex.extend_from_slice(tri) } else { plain.extend_from_slice(tri) }
    }
    (vertices, tex, plain)
}

fn bounds(v: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in v {
        for k in 0..3 {
            mn[k] = mn[k].min(p.pos[k]);
            mx[k] = mx[k].max(p.pos[k]);
        }
    }
    (mn, mx)
}

/// Static far and coarse geometry of every tile, for the scene.
pub struct StreetStatic {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub objects: Vec<GpuObject>,
    pub far_triangles: u64,
    pub coarse_triangles: u64,
}

impl StreetSet {
    /// Tiles, groups and the static far/coarse levels (built on `threads` threads).
    pub fn prepare(world: Arc<StreetWorld>, group_base: u32, palette: Option<StreetPalette>, threads: usize) -> (Self, StreetStatic) {
        let keys = world.tile_keys();
        let chunk = keys.len().div_ceil(threads.max(1)).max(1);
        let built: Vec<(TileMesh, TileMesh)> = std::thread::scope(|s| {
            let hs: Vec<_> = keys
                .chunks(chunk)
                .map(|ks| {
                    let w = &world;
                    s.spawn(move || ks.iter().map(|k| (w.far_tile_mesh(*k), w.coarse_tile_mesh(*k))).collect::<Vec<_>>())
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut st = StreetStatic { vertices: vec![], indices: vec![], objects: vec![], far_triangles: 0, coarse_triangles: 0 };
        let mut tiles = vec![];
        let mut groups = vec![];
        let mut tile_of_key = HashMap::new();
        let tm = eo_street::STREET_TILE_M as f32;
        for (t, (key, (far, coarse))) in keys.iter().zip(built).enumerate() {
            let group = group_base + t as u32;
            let (x0, z0) = (key.0 as f32 * tm, key.1 as f32 * tm);
            let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
            for (mesh, role) in [(&far, LOD_STREET_FAR), (&coarse, LOD_STREET_COARSE)] {
                if mesh.indices.is_empty() {
                    continue;
                }
                let (verts, tex, plain) = to_gpu(mesh, palette.as_ref());
                let base_vertex = st.vertices.len() as i32;
                let (mn, mx) = bounds(&verts);
                ymin = ymin.min(mn[1]);
                ymax = ymax.max(mx[1]);
                st.vertices.extend_from_slice(&verts);
                for (idx, kind) in [(tex, ObjectKind::Street), (plain, ObjectKind::StreetPaint)] {
                    if idx.is_empty() {
                        continue;
                    }
                    let first = st.indices.len() as u32;
                    st.indices.extend_from_slice(&idx);
                    let mut o = GpuObject::new(kind, mn, mx, first, idx.len() as u32, base_vertex);
                    o.material = if kind == ObjectKind::Street { palette.map(|p| p.base).unwrap_or(MATERIAL_VERTEX_COLOUR) } else { MATERIAL_VERTEX_COLOUR };
                    o.lod_group = group;
                    o.lod_role = role;
                    st.objects.push(o);
                    if role == LOD_STREET_FAR {
                        st.far_triangles += idx.len() as u64 / 3;
                    } else {
                        st.coarse_triangles += idx.len() as u64 / 3;
                    }
                }
            }
            if ymin > ymax {
                ymin = 0.0;
                ymax = 0.0;
            }
            // The group's sphere holds the tile square (a near pack's anchors
            // lie in it; its geometry reaches a few metres past).
            let c = [x0 + tm / 2.0, (ymin + ymax) / 2.0, z0 + tm / 2.0];
            let r = 0.5 * ((tm + 24.0).powi(2) * 2.0 + (ymax - ymin + 4.0).powi(2)).sqrt();
            groups.push(GpuLodGroup { center: c, radius: r, ready: 0, detailed: 2, kind: GROUP_KIND_STREET, level: 1 });
            tile_of_key.insert(*key, t as u32);
            tiles.push(StreetTile { key: *key, min: [x0, z0], max: [x0 + tm, z0 + tm] });
        }
        (Self { world, tiles, tile_of_key, group_base, groups, palette }, st)
    }

    pub fn tile_distance(&self, t: usize, cam: Vec3) -> f32 {
        let tile = &self.tiles[t];
        let dx = (tile.min[0] - cam.x).max(0.0).max(cam.x - tile.max[0]);
        let dz = (tile.min[1] - cam.z).max(0.0).max(cam.z - tile.max[1]);
        (dx * dx + dz * dz).sqrt()
    }

    /// the tile's box: its square x the height range its group
    /// sphere was made from (`prepare`: r = 0.5 sqrt(2 (tm + 24)^2 + (dy + 4)^2)).
    pub fn tile_box(&self, t: usize) -> (Vec3, Vec3) {
        let (tile, g) = (&self.tiles[t], &self.groups[t]);
        let tm = eo_street::STREET_TILE_M as f32;
        let h = (4.0 * g.radius * g.radius - 2.0 * (tm + 24.0).powi(2)).max(0.0).sqrt();
        (Vec3::new(tile.min[0], g.center[1] - 0.5 * h, tile.min[1]), Vec3::new(tile.max[0], g.center[1] + 0.5 * h, tile.max[1]))
    }
}

// ---------------------------------------------------------------------------
// Runtime: near packs.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct StreetArena {
    pub vertex_base: u32,
    pub vertex_capacity: u32,
    pub index_base: u32,
    pub index_capacity: u32,
    pub object_base: u32,
    pub object_capacity: u32,
}

pub struct StreetPack {
    pub tile: u32,
    pub vbytes: Vec<u8>,
    pub tex: Vec<u32>,
    pub plain: Vec<u32>,
    pub nv: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub ms: f64,
    pub triangles: u64,
    /// the tile's street decals (when the CC0 decal buffers are attached).
    pub decals: Option<crate::cc0_decals::TileDecals>,
}

#[derive(Clone, Copy, Debug)]
struct Alloc {
    vertex: u32,
    vertices: u32,
    index: u32,
    indices: u32,
    object: u32,
    objects: u32,
}

enum Slot {
    Requested,
    Uploading(Alloc),
    Resident(Alloc),
    /// arrived while the arena was full; asked for again once it frees space.
    Deferred,
}

#[derive(Clone, Debug, Default)]
pub struct StreetStats {
    pub packs_cooked: usize,
    pub packs_uploaded: usize,
    pub packs_evicted: usize,
    pub packs_dropped_unneeded: usize,
    pub packs_skipped_arena_full: usize,
    pub worker_ms: f64,
    pub max_cook_ms: f64,
    pub resident: usize,
    pub resident_triangles: u64,
    pub resident_vertex_bytes: u64,
    pub resident_index_bytes: u64,
    pub group_bytes_written: u64,
    /// decals placed and written, cell overflows, tiles whose slot was taken.
    pub decals_written: u64,
    pub decals_dropped: u64,
    pub decal_slot_busy: u64,
}

pub struct StreetRuntime {
    pub set: Arc<StreetSet>,
    pub arena: StreetArena,
    pub radii: StreetRadii,
    vtx: RangeAlloc,
    idx: RangeAlloc,
    obj: RangeAlloc,
    slots: HashMap<u32, Slot>,
    pending: Arc<Mutex<Vec<u32>>>,
    /// requests and cancellations not yet applied to `pending`.
    outbox: crate::hitch::Outbox<u32>,
    camera: Arc<Mutex<Vec3>>,
    rx: Receiver<StreetPack>,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub stats: StreetStats,
    pub last_group_bytes: u64,
    pub updates: u64,
    /// which tile holds each decal-grid slot (cc0::slot_of).
    decal_slot: Vec<Option<u32>>,
    /// the widened view the workers order by, the near packs'
    /// hysteresis, and the near tiles the last `update` left out as out of view.
    view: Arc<Mutex<Option<crate::view_detail::ViewCone>>>,
    hold: crate::view_detail::ViewHold<u32>,
    pub near_out_of_view: usize,
    /// the arena freed space since a tile was deferred.
    freed: bool,
}

fn tag_of(tile: u32) -> u64 {
    TAG_STREET | tile as u64
}

pub fn cook_pack(set: &StreetSet, tile: u32) -> StreetPack {
    let t0 = Instant::now();
    let key = set.tiles[tile as usize].key;
    let mesh = set.world.cached_mesh(key);
    let (verts, tex, plain) = to_gpu(&mesh, set.palette.as_ref());
    let (min, max) = if verts.is_empty() { ([0.0; 3], [0.0; 3]) } else { bounds(&verts) };
    // the tile's wear decals, placed on the plan of it and its neighbours.
    let decals = crate::cc0::DECALS_WANTED
        .load(Ordering::Relaxed)
        .then(|| crate::cc0_decals::pack(key, eo_street::STREET_TILE_M as f32, crate::cc0_decals::place(&set.world, key)));
    StreetPack {
        decals,
        tile,
        nv: verts.len() as u32,
        vbytes: bytemuck::cast_slice(&verts).to_vec(),
        triangles: (tex.len() + plain.len()) as u64 / 3,
        tex,
        plain,
        min,
        max,
        ms: t0.elapsed().as_secs_f64() * 1e3,
    }
}

impl StreetRuntime {
    pub fn start(set: Arc<StreetSet>, arena: StreetArena, radii: StreetRadii, threads: usize, camera: Vec3) -> Self {
        let (tx, rx): (Sender<StreetPack>, Receiver<StreetPack>) = std::sync::mpsc::channel();
        let pending: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(vec![]));
        let cam = Arc::new(Mutex::new(camera));
        let stop = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let view: Arc<Mutex<Option<crate::view_detail::ViewCone>>> = Arc::new(Mutex::new(None));
        let mut handles = vec![];
        for _ in 0..threads.max(1) {
            let (tx, pending, cam, stop, in_flight, set) = (tx.clone(), pending.clone(), cam.clone(), stop.clone(), in_flight.clone(), set.clone());
            let view = view.clone();
            handles.push(std::thread::spawn(move || {
                crate::hitch::worker_started(); // below the render thread
                while !stop.load(Ordering::Relaxed) {
                    let next = {
                        let c = *cam.lock().unwrap();
                        let v = *view.lock().unwrap();
                        let mut p = pending.lock().unwrap();
                        // in view first by screen importance, then nearest (no view: nearest, as before).
                        let best = p
                            .iter()
                            .enumerate()
                            .map(|(k, &t)| {
                                let (mn, mx) = set.tile_box(t as usize);
                                (k, crate::view_detail::rank(v.as_ref(), mn, mx, set.tile_distance(t as usize, c)))
                            })
                            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        best.map(|(k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            p.swap_remove(k)
                        })
                    };
                    let Some(tile) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(4));
                        continue;
                    };
                    let pack = cook_pack(&set, tile);
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if tx.send(pack).is_err() {
                        break;
                    }
                }
            }));
        }
        Self {
            vtx: RangeAlloc::new(arena.vertex_capacity),
            idx: RangeAlloc::new(arena.index_capacity),
            obj: RangeAlloc::new(arena.object_capacity),
            set,
            arena,
            radii,
            slots: HashMap::new(),
            pending,
            outbox: Default::default(),
            camera: cam,
            rx,
            stop,
            in_flight,
            threads: handles,
            stats: StreetStats::default(),
            last_group_bytes: 0,
            updates: 0,
            decal_slot: vec![None; crate::cc0::SLOTS as usize], // cc0
            view,
            hold: Default::default(),
            near_out_of_view: 0,
            freed: false,
        }
    }

    /// whether the tile's near pack is resident (its group's ready bit).
    pub fn is_resident(&self, tile: u32) -> bool {
        matches!(self.slots.get(&tile), Some(Slot::Resident(_)))
    }

    pub fn begin_frame(&mut self) {
        self.last_group_bytes = 0;
    }

    pub fn update(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, uploads: &mut UploadQueue, cam: Vec3) {
        self.updates += 1;
        crate::hitch::try_set(&self.camera, cam); // never wait for a worker's lock
        // the far strips are static (radial); a near pack is wanted
        // only while its tile is in the widened view (or was, within the hold).
        let cone = renderer.view_cone();
        crate::hitch::try_set(&self.view, cone);
        self.near_out_of_view = 0;
        // a deferred tile (the arena was full) is asked for again
        // once space freed; while one waits, nothing out of view is held and
        // nothing past the load radius is kept.
        if self.freed {
            self.slots.retain(|_, s| !matches!(s, Slot::Deferred));
            self.freed = false;
        }
        let under = self.slots.values().any(|s| matches!(s, Slot::Deferred));
        let r = self.radii;
        let tm = eo_street::STREET_TILE_M;
        let reach = (r.near_drop as f64 / tm).ceil() as i32 + 1;
        let (cx, cz) = eo_street::tile_key(cam.x as f64, cam.z as f64);
        let mut wanted: HashSet<u32> = HashSet::new();
        for tz in cz - reach..=cz + reach {
            for tx in cx - reach..=cx + reach {
                let Some(&t) = self.set.tile_of_key.get(&(tx, tz)) else { continue };
                let d = self.set.tile_distance(t as usize, cam);
                if d <= r.near_load || (self.slots.contains_key(&t) && d <= r.near_drop && !under) {
                    if let Some(c) = cone.as_ref() {
                        let (mn, mx) = self.set.tile_box(t as usize);
                        let held = self.hold.wanted(c, t, mn, mx);
                        if !held || (under && !c.sees_box(mn, mx)) {
                            self.near_out_of_view += 1;
                            continue;
                        }
                    }
                    wanted.insert(t);
                }
            }
        }
        if let Some(c) = cone.as_ref() {
            self.hold.prune(c);
        }
        let mut evictable = 0;
        let gone: Vec<u32> = self
            .slots
            .iter()
            .filter(|(k, _)| !wanted.contains(k))
            .filter(|(_, s)| {
                if matches!(s, Slot::Resident(_)) {
                    evictable += 1;
                    evictable <= crate::kit::EVICTIONS_PER_FRAME
                } else {
                    true
                }
            })
            .map(|(k, _)| *k)
            .collect();
        if !gone.is_empty() {
            let gone_set: HashSet<u32> = gone.iter().copied().collect();
            for k in &gone_set {
                self.outbox.cancel(*k); // applied without waiting for the workers' lock
            }
            uploads.cancel(|tag| tag & TAG_STREET == 0 || tag & crate::kit::TAG_KIT != 0 || !gone_set.contains(&((tag & 0xffff_ffff) as u32)));
            for k in gone {
                match self.slots.remove(&k) {
                    Some(Slot::Resident(a)) => {
                        self.evict(renderer, queue, k, a);
                        self.stats.packs_evicted += 1;
                    }
                    Some(Slot::Uploading(a)) => {
                        self.free(a);
                        self.stats.packs_dropped_unneeded += 1;
                    }
                    _ => {}
                }
                self.release_decals(renderer, queue, k); // cc0
            }
        }
        for k in &wanted {
            if !self.slots.contains_key(k) {
                self.slots.insert(*k, Slot::Requested);
                self.outbox.request(*k); // hitch
            }
        }
        self.outbox.flush(&self.pending); // try_lock only
        while let Ok(mut pack) = self.rx.try_recv() {
            crate::hitch::count(crate::hitch::Event::Arrival, 1); // hitch
            self.stats.worker_ms += pack.ms;
            self.stats.max_cook_ms = self.stats.max_cook_ms.max(pack.ms);
            self.stats.packs_cooked += 1;
            if !matches!(self.slots.get(&pack.tile), Some(Slot::Requested)) {
                self.stats.packs_dropped_unneeded += 1;
                continue;
            }
            match self.make_job(&mut pack) {
                Some((job, a)) => {
                    self.slots.insert(pack.tile, Slot::Uploading(a));
                    uploads.push(job);
                }
                None => {
                    self.slots.insert(pack.tile, Slot::Deferred); // until space frees (was: asked for again every frame)
                    self.stats.packs_skipped_arena_full += 1;
                }
            }
        }
    }

    fn free(&mut self, a: Alloc) {
        self.vtx.free(a.vertex, a.vertices);
        self.idx.free(a.index, a.indices);
        self.obj.free(a.object, a.objects);
        self.freed = true;
    }

    fn make_job(&mut self, p: &mut StreetPack) -> Option<(Job, Alloc)> {
        let ni = (p.tex.len() + p.plain.len()) as u32;
        let objects_needed = 2;
        let v = self.vtx.alloc(p.nv)?;
        let Some(i) = self.idx.alloc(ni) else {
            self.vtx.free(v, p.nv);
            return None;
        };
        let Some(o) = self.obj.alloc(objects_needed) else {
            self.vtx.free(v, p.nv);
            self.idx.free(i, ni);
            return None;
        };
        let a = Alloc { vertex: v, vertices: p.nv, index: i, indices: ni, object: o, objects: objects_needed };
        let ar = self.arena;
        let group = self.set.group_base + p.tile;
        let base_vertex = (ar.vertex_base + v) as i32;
        let n_tex = p.tex.len() as u32;
        let mut objects = vec![];
        for (first, count, kind) in [(0u32, n_tex, ObjectKind::Street), (n_tex, p.plain.len() as u32, ObjectKind::StreetPaint)] {
            let mut ob = GpuObject::new(kind, p.min, p.max, ar.index_base + i + first, count, base_vertex);
            ob.material = if kind == ObjectKind::Street { self.set.palette.map(|q| q.base).unwrap_or(MATERIAL_VERTEX_COLOUR) } else { MATERIAL_VERTEX_COLOUR };
            ob.lod_group = group;
            ob.lod_role = LOD_STREET_NEAR;
            objects.push(ob);
        }
        let mut idx = std::mem::take(&mut p.tex);
        idx.extend_from_slice(&p.plain);
        let vs = std::mem::size_of::<Vertex>() as u64;
        let os = std::mem::size_of::<GpuObject>() as u64;
        let mut bulk = vec![
            Write { target: Target::Vertex, offset: (ar.vertex_base + v) as u64 * vs, data: Data::Bytes(std::mem::take(&mut p.vbytes)) },
            Write { target: Target::Index, offset: (ar.index_base + i) as u64 * 4, data: Data::Words(idx) },
        ];
        let mut finish = vec![Write { target: Target::Object, offset: (ar.object_base + o) as u64 * os, data: Data::Objects(objects) }];
        // the decal records and cells with the pack, the slot header last (nothing reads a half-written slot).
        if let Some(d) = p.decals.take() {
            let key = self.set.tiles[p.tile as usize].key;
            let slot = crate::cc0::slot_of(key) as usize;
            match self.decal_slot[slot] {
                Some(t) if t != p.tile => self.stats.decal_slot_busy += 1,
                _ => {
                    self.decal_slot[slot] = Some(p.tile);
                    self.stats.decals_written += d.count as u64;
                    self.stats.decals_dropped += d.dropped as u64;
                    let s = slot as u32;
                    bulk.push(Write { target: Target::Cc0Decals, offset: crate::cc0::decals_offset(s), data: Data::Bytes(bytemuck::cast_slice(&d.records).to_vec()) });
                    bulk.push(Write { target: Target::Cc0Grid, offset: crate::cc0::cells_offset(s), data: Data::Bytes(bytemuck::cast_slice(&d.cells).to_vec()) });
                    if crate::cc0::LIFE_CELLS.load(Ordering::Relaxed) {
                        // street life (`--street-life`): its own cells (empty ones over a tile without it)
                        let life = if d.life_cells.is_empty() { vec![[0xffff_ffffu32; 4]; crate::cc0_decals::CELLS * crate::cc0_decals::CELLS] } else { d.life_cells.clone() };
                        bulk.push(Write { target: Target::Cc0Grid, offset: crate::cc0::life_cells_offset(s), data: Data::Bytes(bytemuck::cast_slice(&life).to_vec()) });
                    }
                    finish.push(Write { target: Target::Cc0Grid, offset: crate::cc0::header_offset(s), data: Data::Words(crate::cc0::header(key, d.count as u32).to_vec()) });
                }
            }
        }
        let job = Job::new(tag_of(p.tile), bulk, finish);
        Some((job, a))
    }

    /// free a tile's decal-grid slot (its header emptied) if it holds one.
    fn release_decals(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32) {
        let key = self.set.tiles[tile as usize].key;
        let slot = crate::cc0::slot_of(key) as usize;
        if self.decal_slot[slot] == Some(tile) {
            self.decal_slot[slot] = None;
            renderer.write(queue, Target::Cc0Grid, crate::cc0::header_offset(slot as u32), bytemuck::cast_slice(&crate::cc0::empty_header()));
            self.last_group_bytes += 16;
        }
    }

    fn write_group(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32, ready: bool) {
        // `ready` is the fifth word of the 32-byte group.
        let at = (self.set.group_base as u64 + tile as u64) * std::mem::size_of::<GpuLodGroup>() as u64 + 16;
        renderer.write(queue, Target::Group, at, bytemuck::bytes_of(&(ready as u32)));
        self.last_group_bytes += 4;
        self.stats.group_bytes_written += 4;
    }

    fn evict(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, tile: u32, a: Alloc) {
        crate::hitch::count(crate::hitch::Event::Eviction, 1); // hitch
        self.write_group(renderer, queue, tile, false);
        let zero = vec![GpuObject::zeroed_street(); a.objects as usize];
        renderer.write(queue, Target::Object, (self.arena.object_base + a.object) as u64 * std::mem::size_of::<GpuObject>() as u64, bytemuck::cast_slice(&zero));
        self.last_group_bytes += (zero.len() * std::mem::size_of::<GpuObject>()) as u64;
        self.account(a, false);
        self.free(a);
    }

    fn account(&mut self, a: Alloc, add: bool) {
        let tris = a.indices as u64 / 3;
        let vb = a.vertices as u64 * std::mem::size_of::<Vertex>() as u64;
        let ib = a.indices as u64 * 4;
        if add {
            self.stats.resident += 1;
            self.stats.resident_triangles += tris;
            self.stats.resident_vertex_bytes += vb;
            self.stats.resident_index_bytes += ib;
        } else {
            self.stats.resident -= 1;
            self.stats.resident_triangles -= tris;
            self.stats.resident_vertex_bytes -= vb;
            self.stats.resident_index_bytes -= ib;
        }
    }

    pub fn after_pump(&mut self, renderer: &crate::Renderer, queue: &wgpu::Queue, done: &[u64]) {
        for &tag in done {
            if tag & TAG_STREET == 0 || tag & crate::kit::TAG_KIT != 0 {
                continue;
            }
            let tile = (tag & 0xffff_ffff) as u32;
            let Some(Slot::Uploading(a)) = self.slots.remove(&tile) else { continue };
            self.slots.insert(tile, Slot::Resident(a));
            crate::hitch::mark(crate::hitch::Mark::StreetTileResident, 1);
            self.write_group(renderer, queue, tile, true);
            self.account(a, true);
            self.stats.packs_uploaded += 1;
        }
    }

    pub fn settled(&self) -> bool {
        self.updates > 0
            && self.outbox.is_empty() // hitch
            && self.pending.lock().unwrap().is_empty()
            && self.in_flight.load(Ordering::SeqCst) == 0
            && self.slots.values().all(|s| matches!(s, Slot::Resident(_) | Slot::Deferred)) // + deferred
    }

    pub fn arena_use(&self) -> (u32, u32, u32, u32, u32, u32) {
        (self.vtx.used, self.vtx.peak, self.idx.used, self.idx.peak, self.obj.used, self.obj.peak)
    }

    /// Resident tiles, for tests: tile -> object range.
    pub fn resident(&self) -> Vec<(u32, (u32, u32))> {
        self.slots
            .iter()
            .filter_map(|(k, s)| if let Slot::Resident(a) = s { Some((*k, (self.arena.object_base + a.object, a.objects))) } else { None })
            .collect()
    }
}

impl Drop for StreetRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

impl GpuObject {
    /// An empty street-arena slot (never drawn).
    pub fn zeroed_street() -> Self {
        let mut o = GpuObject::new(ObjectKind::Street, [0.0; 3], [0.0; 3], 0, 0, 0);
        o.lod_role = LOD_STREET_NEAR;
        o.lod_group = 0;
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two carriageway quads meeting at a ridge (a camber's two strips) and a
    /// kerb face beside them: the shared corners take the averaged normal, each
    /// far corner its own quad's, and the kerb face keeps its flat one.
    #[test]
    fn the_carriageway_shades_smooth_and_the_kerb_keeps_its_edge() {
        let mut m = TileMesh::default();
        // strip A rises 2 % toward x = 1, strip B falls from it.
        let pa = [[0.0, 0.0, 0.0], [1.0, 0.02, 0.0], [1.0, 0.02, 1.0], [0.0, 0.0, 1.0]];
        let pb = [[1.0, 0.02, 0.0], [2.0, 0.0, 0.0], [2.0, 0.0, 1.0], [1.0, 0.02, 1.0]];
        let na = [-0.02f32, 1.0, 0.0];
        let nb = [0.02f32, 1.0, 0.0];
        for (p, n, layer) in [(pa, na, Layer::Carriageway), (pb, nb, Layer::Carriageway), ([[0.0, 0.0, 0.0], [0.0, 0.15, 0.0], [0.0, 0.15, 1.0], [0.0, 0.0, 1.0]], [-1.0, 0.0, 0.0], Layer::CurbFace)] {
            let b = m.positions.len() as u32;
            for q in p {
                m.positions.push(q);
                m.normals.push(n);
                m.colors.push(0);
                m.kerb.push(-1.0);
            }
            m.indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
            m.layers.extend_from_slice(&[layer, layer]);
        }
        let s = smooth_carriageway(&m).expect("a carriageway");
        assert!(s[1][0].abs() < 1e-5 && (s[1][1] - 1.0).abs() < 1e-5, "the ridge's corner is level: {:?}", s[1]);
        assert!(s[0][0] < -0.015, "strip A's outer corner keeps its fall: {:?}", s[0]);
        assert_eq!(s[8], [-1.0, 0.0, 0.0], "the kerb face keeps its normal");
        SMOOTH_CARRIAGEWAY.store(false, Ordering::Relaxed);
        let (v_off, _, _) = to_gpu(&m, None);
        assert_eq!(v_off[1].normal & 0x00ff_ffff, eo_assets::glb::pack_normal(Vec3::from(na)) & 0x00ff_ffff, "off: each primitive's own normal, as before");
    }
}
