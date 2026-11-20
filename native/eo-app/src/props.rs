//! the street props at run time. At start, on a thread: the props
//! world (eo-props: the roadway index, the tree and sign inventories over the
//! street world) and the props library (eo-render `props_lib`: the JS GLBs,
//! hash-verified, cooked into `cache/props/`). Then tiles stream around the
//! camera: worker threads (below the render thread) plan a 128 m street tile
//! (`PropsWorld::plan_tile`) and turn it into GPU instances, resolving every
//! signal and pedestrian head to eo-sim's signal controller of its junction;
//! the render thread allocates an arena range and queues the instance bytes
//! on the shared budgeted upload queue (`Target::PropInstances`). Each frame
//! the lamp channels of the resident heads are evaluated with the
//! simulation's own controller functions on the simulation clock
//! (`SignalJunction::aspect_at`, `ped_indication`) and written (a few hundred
//! words); the parked cars of the resident tiles nearest the camera (the JS
//! `VEHICLE_RINGS` radii and caps) go to the agent renderer as static
//! instances. `--no-props` turns all of it off.

mod hill_rock; // the hill rock's mid ring past the props tiles
pub(crate) mod vegetation; // the Golden Gate's coastal trees, ground cover and hill rock on the props tiles

use eo_props::{Live, Model, ParkedCar, PropsWorld, TilePlacement};
use eo_render::props::{EMPTY, GpuLampLight, GpuPropInstance, GpuWireSeg, INSTANCE_CAPACITY, LAMP_CAPACITY, LIGHT_CAPACITY, WIRE_CAPACITY};
use eo_render::props_lib::PropLibrary;
use eo_render::props_lib2 as lib2;
use eo_render::upload::{Data, Job, Target, UploadQueue, Write};
use eo_render::{GpuContext, Renderer};
use eo_sim::signals::SignalJunction;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const TAG_PROPS: u64 = 1 << 58;
/// with TAG_PROPS: an upload of a vegetation ring's cell (not a props tile): the
/// fine ring (grass) and the mid ring (hill rock past the props tiles).
pub const TAG_FINE: u64 = 1 << 40;
pub const TAG_MID: u64 = 1 << 41;
/// the vegetation rings (`VegRing`): 1 fine, 2 mid; 0 is a props tile.
const RING_FINE: u8 = 1;
const RING_MID: u8 = 2;

/// One vegetation ring's requests and residents (its cells are planted by the props workers).
#[derive(Default)]
struct VegRing {
    pending: Arc<Mutex<Vec<Key>>>,
    outbox: eo_render::hitch::Outbox<Key>,
    slots: HashMap<Key, Slot>,
}
/// Tiles load within this of the camera (3D distance to the tile's ground
/// rectangle) and drop past the second: the street trees' far ring (the
/// JS window ring is 260 m; past it only silhouettes) with margin.
pub const LOAD_M: f32 = 300.0;
pub const DROP_M: f32 = 360.0;
pub const WORKERS: usize = 3;

type Key = (i32, i32);

/// what of the props2 work is on (A/B flags; default everything).
#[derive(Clone, Copy, Debug)]
pub struct Props2Args {
    /// The street-furniture walk's kinds (hydrants ... signal cabinets) and the lamp lights.
    pub furniture: bool,
    /// ez-tree street trees (off: the card tree at the species' size).
    pub eztree: bool,
    /// The street overhead's poles, cabinets, sign pickets, corner signs and lanterns.
    pub overhead: bool,
    /// The overhead's wires.
    pub wires: bool,
    /// Shader features (`eo_render::props::F_*`: fade, wind, leaf light, domes, sign legends).
    pub features: u32,
    /// `--capture-props2`: the props2 capture cameras (bench `--capture`).
    pub capture: bool,
}

impl Default for Props2Args {
    fn default() -> Self {
        Self { furniture: true, eztree: true, overhead: true, wires: true, features: eo_render::props::F_ALL, capture: false }
    }
}

impl Props2Args {
    /// One command-line flag; false when it is not a props2 flag.
    pub fn parse_flag(&mut self, s: &str) -> bool {
        match s {
            "--no-props2" => {
                *self = Self { furniture: false, eztree: false, overhead: false, wires: false, features: 0, capture: self.capture };
            }
            "--no-furniture" => self.furniture = false,
            "--no-eztree" => self.eztree = false,
            "--no-overhead" => {
                self.overhead = false;
                self.wires = false;
            }
            "--no-wires" => self.wires = false,
            "--no-props2-shading" => self.features = 0,
            "--no-fade" => self.features &= !eo_render::props::F_FADE,
            "--no-wind" => self.features &= !eo_render::props::F_WIND,
            "--capture-props2" => self.capture = true,
            _ => return false,
        }
        true
    }
}

/// the library facts the tile workers need.
#[derive(Clone, Default)]
struct LibFacts {
    species: usize,
    tree_dims: Vec<[f32; 2]>,
    sign_variants: usize,
    /// the ground the vegetation and hill rock stand on (None: none planted).
    veg: Option<Arc<vegetation::VegGround>>,
}

#[derive(Clone)]
enum LiveRef {
    Dark,
    Vehicle { j: Arc<SignalJunction>, axis: usize },
    Pedestrian { j: Arc<SignalJunction>, crossed: Option<usize>, length: f64 },
}

struct TilePack {
    key: Key,
    /// 0 a props tile, else the vegetation ring (`RING_FINE`, `RING_MID`) the cell is of.
    ring: u8,
    instances: Vec<GpuPropInstance>,
    /// Index into `instances` and its live source.
    lives: Vec<(u32, LiveRef)>,
    parked: Vec<ParkedCar>,
    counts: HashMap<Model, usize>,
    ms: f64,
    /// the overhead's wire segments and the district lights' luminaires.
    wires: Vec<GpuWireSeg>,
    lights: Vec<GpuLampLight>,
    overhead_ms: f64,
    extra_counts: HashMap<&'static str, usize>,
}

enum Slot {
    Requested,
    Resident {
        at: u32,
        n: u32,
        lamps: Vec<u32>,
        parked: Arc<Vec<ParkedCar>>,
        counts: HashMap<Model, usize>,
        // wire and light ranges, extra counts.
        wire_at: u32,
        wire_n: u32,
        light_at: u32,
        light_n: u32,
        extra: HashMap<&'static str, usize>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct PropsStats {
    pub tiles_planned: usize,
    pub plan_ms_total: f64,
    pub plan_ms_max: f64,
    pub peak_slots: u32,
    pub lamp_channels: usize,
    pub parked_submitted: usize,
    pub bytes_queued: u64,
    pub ready_ms: Option<f64>,
    pub pump_ms_max: f64,
    pub overhead_ms_total: f64,
    pub overhead_ms_max: f64,
    pub peak_wire_slots: u32,
    pub peak_light_slots: u32,
}

struct Loaded {
    world: Arc<PropsWorld>,
    lib: PropLibrary,
    source: Arc<eo_sim::native::NativeSource>,
    facts: LibFacts,
    wire_look: eo_render::props::WireLook,
}

pub struct PropsRuntime {
    pub on: bool,
    rx: Option<Receiver<Result<Loaded, String>>>,
    #[allow(clippy::type_complexity)]
    gpu_rx: Option<Receiver<Result<(eo_render::props::PropsGpu, Arc<PropsWorld>, Arc<eo_sim::native::NativeSource>, LibFacts), String>>>,
    p2: Props2Args,
    wire_alloc: eo_render::kit::RangeAlloc,
    light_alloc: eo_render::kit::RangeAlloc,
    light_count_written: Option<u32>,
    world: Option<Arc<PropsWorld>>,
    source: Option<Arc<eo_sim::native::NativeSource>>,
    pending: Arc<Mutex<Vec<Key>>>,
    outbox: eo_render::hitch::Outbox<Key>,
    /// the vegetation's rings: [fine (grass cells near the eye), mid (hill rock past the props tiles)].
    rings: [VegRing; 2],
    veg: Option<Arc<vegetation::VegGround>>,
    camera: Arc<Mutex<Vec3>>,
    /// the widened view the tile workers order by (tiles stay radial).
    view: Arc<Mutex<Option<eo_render::view_detail::ViewCone>>>,
    /// the cooked tiles, each with the vegetation epoch it was cooked on (`veg_epoch`).
    tiles_rx: Option<Receiver<(TilePack, u32)>>,
    /// the vegetation epoch the resident fine cells were cooked on.
    veg_epoch: u32,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    slots: HashMap<Key, Slot>,
    alloc: eo_render::kit::RangeAlloc,
    lamp_free: Vec<u32>,
    lamp_next: u32,
    lamp_src: HashMap<u32, LiveRef>,
    lamp_words: Vec<u32>,
    parked_cam: Option<Vec3>,
    parked_dirty: bool,
    pub parked: Vec<eo_agents::GpuAgent>,
    pub stats: PropsStats,
    pub errors: Vec<String>,
    started: Instant,
    clock: f64,
}

impl PropsRuntime {
    pub fn start(world: &crate::World, args: &crate::Args) -> Self {
        let mut rt = PropsRuntime {
            on: args.props,
            rx: None,
            gpu_rx: None,
            p2: args.props2,
            wire_alloc: eo_render::kit::RangeAlloc::new(WIRE_CAPACITY),
            light_alloc: eo_render::kit::RangeAlloc::new(LIGHT_CAPACITY),
            light_count_written: None,
            world: None,
            source: None,
            pending: Arc::new(Mutex::new(vec![])),
            outbox: Default::default(),
            rings: Default::default(),
            veg: None,
            camera: Arc::new(Mutex::new(Vec3::ZERO)),
            view: Arc::new(Mutex::new(None)),
            tiles_rx: None,
            veg_epoch: veg_epoch(),
            stop: Arc::new(AtomicBool::new(false)),
            in_flight: Arc::new(AtomicUsize::new(0)),
            threads: vec![],
            slots: HashMap::new(),
            alloc: eo_render::kit::RangeAlloc::new(INSTANCE_CAPACITY),
            lamp_free: vec![],
            lamp_next: 0,
            lamp_src: HashMap::new(),
            lamp_words: vec![],
            parked_cam: None,
            parked_dirty: true,
            parked: vec![],
            stats: PropsStats::default(),
            errors: vec![],
            started: Instant::now(),
            clock: 0.0,
        };
        let Some(street) = world.street.as_ref().map(|s| s.world.clone()) else {
            rt.on = false;
            return rt;
        };
        if !rt.on {
            return rt;
        }
        let paths = world.paths.clone();
        let cache = crate::cache_dir(args);
        // the vegetation's ground: the scene's terrain vertices, copied now (the loader builds the rest).
        let veg_switches = eo_landmarks::vegetation::switches();
        let veg_grid = if veg_switches.any() {
            let t = Instant::now();
            let g = vegetation::TerrainGrid::from_scene(&world.scene, &world.elevation.meta);
            if std::env::var("EO_TRACE_LOAD").is_ok() {
                eprintln!("vegetation: terrain grid copied in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
            }
            g
        } else {
            None
        };
        let veg_water = world.water.clone();
        let city = world.city.clone(); // the furniture frontage and the service drops read the loaded buildings
        let p2 = args.props2;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("eo-props-load".into())
            .spawn(move || {
                eo_render::hitch::worker_started();
                let r = (|| -> anyhow::Result<Loaded> {
                    let js = Arc::new(eo_props::JsProps::load()?);
                    let ca = eo_data::CaStandards::load(&paths)?;
                    let std = eo_props::Standards::load(&ca)?;
                    let lens = ca.metric_key("signal-lens-size", "circularNewFaces")?;
                    // the ez-tree bake, the sign atlases, the overhead export, the domes (all cached, off this thread's caller).
                    let t_lib = Instant::now();
                    let cache_root = cache.clone().unwrap_or_else(eo_assets::default_cache_dir);
                    let ez = eo_props::eztree::cook_all(Some(&cache_root)).map_err(|e| eprintln!("props2: ez-tree bake failed ({e:#}); card trees drawn")).ok();
                    let signs = eo_props::sign_atlas::Fonts::system()
                        .and_then(|f| eo_props::sign_atlas::load_or_cook(&cache_root, &js, &f, &[], eo_props::sign_atlas::SIGN_FALLBACK_STREETS))
                        .map(|(a, _)| a)
                        .map_err(|e| eprintln!("props2: sign atlases not cooked ({e:#}); plates draw their face colour"))
                        .ok();
                    let overhead_js = eo_props::overhead::OverheadJs::load().map_err(|e| eprintln!("props2: overhead export not read ({e:#})")).ok();
                    let dome = lib2::dome_standards(&ca)?;
                    let extras = lib2::LibExtras { ez: ez.as_deref(), signs: signs.as_ref(), overhead: overhead_js.as_ref() };
                    let lib = PropLibrary::load_full(&paths.repo_root, &js, lens, &extras, dome, cache.as_deref())?;
                    for m in &lib.missing {
                        eprintln!("props: missing asset, proxy drawn: {m}");
                    }
                    let library_ms = t_lib.elapsed().as_secs_f64() * 1e3;
                    let source = Arc::new(eo_sim::native::load(&paths)?);
                    let world = Arc::new(PropsWorld::new(street, js, std));
                    if p2.furniture || p2.eztree {
                        world.install_furniture(&city.buildings)?;
                    }
                    if p2.overhead || p2.wires {
                        eo_props::overhead::install(&world, eo_props::overhead::DropBuilding::from_city(&city.buildings))?;
                    }
                    let wire_look = overhead_js
                        .as_ref()
                        .map(|o| {
                            let h = |k: &str, d: f32| o.consts["streetOverhead"][k].as_f64().map(|v| v as f32).unwrap_or(d);
                            let w = eo_render::props::WireLook::default();
                            eo_render::props::WireLook {
                                pixel_half_width_rad: h("pixelHalfWidthRad", w.pixel_half_width_rad),
                                fade_near_m: h("fadeNearM", w.fade_near_m),
                                fade_far_m: h("fadeFarM", w.fade_far_m),
                                min_coverage: h("minCoverage", w.min_coverage),
                            }
                        })
                        .unwrap_or_default();
                    let veg = veg_grid.map(|g| {
                        let land = eo_data::LandCover::load(&paths).map_err(|e| eprintln!("vegetation: land cover not read ({e:#}); classes from the greens only")).ok();
                        Arc::new(vegetation::VegGround::new(g, world.street.clone(), land, veg_water, veg_switches, &paths.repo_root))
                    });
                    let facts = LibFacts { species: lib.tree_species.len(), tree_dims: lib.tree_dims.clone(), sign_variants: overhead_js.as_ref().map(|o| o.sign_variants.len()).unwrap_or(0), veg };
                    eprintln!("props2: library ready in {library_ms:.0} ms ({} meshes, {} tree species, {} sign variants)", lib.meshes.len(), facts.species, facts.sign_variants);
                    Ok(Loaded { world, lib, source, facts, wire_look })
                })();
                let _ = tx.send(r.map_err(|e| format!("{e:#}")));
            })
            .expect("spawn props loader");
        rt.rx = Some(rx);
        rt
    }

    fn start_workers(&mut self, world: Arc<PropsWorld>, source: Arc<eo_sim::native::NativeSource>, facts: LibFacts) {
        let (tx, rx): (Sender<(TilePack, u32)>, Receiver<(TilePack, u32)>) = std::sync::mpsc::channel();
        let junctions: Arc<Mutex<HashMap<eo_sim::TileKey, Arc<Vec<Arc<SignalJunction>>>>>> = Arc::new(Mutex::new(HashMap::new()));
        let p2 = self.p2;
        for _ in 0..WORKERS {
            let (tx, pending, cam, stop, in_flight, world, source, junctions, facts) =
                (tx.clone(), self.pending.clone(), self.camera.clone(), self.stop.clone(), self.in_flight.clone(), world.clone(), source.clone(), junctions.clone(), facts.clone());
            let view = self.view.clone();
            let ring_pending = [self.rings[0].pending.clone(), self.rings[1].pending.clone()];
            self.threads.push(std::thread::spawn(move || {
                eo_render::hitch::worker_started(); // below the render thread
                while !stop.load(Ordering::Relaxed) {
                    let next = {
                        let c = *cam.lock().unwrap();
                        let v = *view.lock().unwrap();
                        let mut p = pending.lock().unwrap();
                        // tiles in view first by screen importance, then nearest (no view: nearest, as before).
                        let best = p
                            .iter()
                            .enumerate()
                            .map(|(k, t)| {
                                let d = tile_distance(*t, c);
                                let r = match v.as_ref() {
                                    None => d,
                                    Some(cone) => {
                                        let (mn, mx) = tile_box(*t, |x, z| world.street.terrain.ground(x as f64, z as f64) as f32);
                                        eo_render::view_detail::rank(Some(cone), mn, mx, d)
                                    }
                                };
                                (k, r)
                            })
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        // the vegetation's fine cells compete by their own distance (grass near the eye first).
                        // (the render thread only ever try-locks these lists, so a worker may wait on them.)
                        // the mid ring's rock after both (its cells are far by construction).
                        let mut f = ring_pending[0].lock().unwrap();
                        let mut m = ring_pending[1].lock().unwrap();
                        let fine_best = f.iter().enumerate().map(|(k, t)| (k, fine_distance(*t, c) * 2.0)).min_by(|a, b| a.1.total_cmp(&b.1));
                        let mid_best = m.iter().enumerate().map(|(k, t)| (k, tile_distance(*t, c) * 3.0)).min_by(|a, b| a.1.total_cmp(&b.1));
                        let mut pick: Option<(u8, usize, f32)> = best.map(|(k, r)| (0, k, r));
                        for (ring, cand) in [(RING_FINE, fine_best), (RING_MID, mid_best)] {
                            if let Some((k, r)) = cand {
                                if pick.is_none_or(|p| r < p.2) {
                                    pick = Some((ring, k, r));
                                }
                            }
                        }
                        pick.map(|(ring, k, _)| {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            let key = match ring {
                                RING_FINE => f.swap_remove(k),
                                RING_MID => m.swap_remove(k),
                                _ => p.swap_remove(k),
                            };
                            (key, ring)
                        })
                    };
                    let Some((key, ring)) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(4));
                        continue;
                    };
                    let epoch = veg_epoch();
                    let pack = match ring {
                        RING_FINE => cook_fine(facts.veg.as_deref(), key),
                        RING_MID => hill_rock::cook_mid(facts.veg.as_deref(), key),
                        _ => cook(&world, &source, &junctions, key, &p2, &facts),
                    };
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if tx.send((pack, epoch)).is_err() {
                        break;
                    }
                }
            }));
        }
        self.tiles_rx = Some(rx);
    }

    /// Every wanted tile resident (bench settle).
    pub fn settled(&self) -> bool {
        !self.on
            || (self.world.is_some()
                && self.outbox.is_empty()
                && self.pending.lock().unwrap().is_empty()
                && self.in_flight.load(Ordering::SeqCst) == 0
                && self.veg_epoch == veg_epoch()
                && self.slots.values().all(|s| matches!(s, Slot::Resident { .. }))
                && self.rings.iter().all(|r| r.outbox.is_empty() && r.pending.lock().unwrap().is_empty() && r.slots.values().all(|s| matches!(s, Slot::Resident { .. }))))
    }

    /// Once a frame on the render thread, before `Renderer::update`.
    /// `sim_time`: the simulation clock the signal controllers run on (None: own clock).
    pub fn pump(&mut self, ctx: &GpuContext, renderer: &Renderer, uploads: &mut UploadQueue, eye: Vec3, sim_time: Option<f64>, dt: f64) {
        let t0 = Instant::now();
        if let Some(p) = renderer.props.get() {
            p.set_enabled(self.on);
        }
        if !self.on {
            return;
        }
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                match r {
                    Ok(l) => {
                        // The props renderer (pipelines, buffers, texture mips) is
                        // built on a thread of its own, never on this one.
                        let init = renderer.props_init(ctx);
                        let (tx, rx) = std::sync::mpsc::channel();
                        let Loaded { world, lib, source, facts, wire_look } = l;
                        let features = self.p2.features;
                        std::thread::Builder::new()
                            .name("eo-props-gpu".into())
                            .spawn(move || {
                                eo_render::hitch::worker_started();
                                let r = eo_render::props::PropsGpu::new(&init, &lib).map_err(|e| format!("{e:#}"));
                                let _ = tx.send(r.map(|mut g| {
                                    g.set_wire_look(wire_look);
                                    g.set_features(features);
                                    (g, world, source, facts)
                                }));
                            })
                            .expect("spawn props gpu builder");
                        self.gpu_rx = Some(rx);
                    }
                    Err(e) => {
                        eprintln!("props: load failed: {e}");
                        self.errors.push(e);
                        self.on = false;
                        return;
                    }
                }
            }
        }
        if let Some(rx) = &self.gpu_rx {
            if let Ok(r) = rx.try_recv() {
                self.gpu_rx = None;
                match r {
                    Ok((g, world, source, facts)) => {
                        renderer.attach_props_gpu(g);
                        self.veg = facts.veg.clone();
                        self.start_workers(world.clone(), source.clone(), facts);
                        self.world = Some(world);
                        self.source = Some(source);
                        self.stats.ready_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
                    }
                    Err(e) => {
                        eprintln!("props: renderer failed: {e}");
                        self.errors.push(e);
                        self.on = false;
                        return;
                    }
                }
            }
        }
        let Some(world) = self.world.clone() else { return };
        let Some(gpu) = renderer.props.get() else { return };
        eo_render::hitch::try_set(&self.camera, eye); // never wait for a worker's lock
        eo_render::hitch::try_set(&self.view, renderer.view_cone()); // request order only
        // Wanted tiles.
        let tm = eo_street::STREET_TILE_M as f32;
        let reach = (DROP_M / tm).ceil() as i32 + 1;
        let (cx, cz) = eo_street::tile_key(eye.x as f64, eye.z as f64);
        let mut wanted: HashSet<Key> = HashSet::new();
        for tz in cz - reach..=cz + reach {
            for tx in cx - reach..=cx + reach {
                let k = (tx, tz);
                // a street tile, or (the vegetation) land without one.
                if !world.street.tiles.contains_key(&k) && !self.veg.as_ref().is_some_and(|v| v.tiles.contains(&k)) {
                    continue;
                }
                // the true 3D distance to the tile's ground (nothing in a tile farther
                // than the longest draw distance, 360 m in 3D, can be drawn; from the elevated
                // path most of a 300 m horizontal disc is past it and was planned for nothing).
                let d = tile_distance_3d(k, eye, |x, z| world.street.terrain.ground(x as f64, z as f64) as f32);
                if d <= LOAD_M || (self.slots.contains_key(&k) && d <= DROP_M) {
                    wanted.insert(k);
                }
            }
        }
        let gone: Vec<Key> = self.slots.keys().filter(|k| !wanted.contains(k)).copied().collect();
        if !gone.is_empty() {
            let gone_set: HashSet<Key> = gone.iter().copied().collect();
            for k in &gone_set {
                self.outbox.cancel(*k);
            }
            uploads.cancel(|tag| tag & TAG_PROPS == 0 || tag & (eo_render::kit::TAG_KIT | eo_render::street::TAG_STREET | (1 << 60) | (1 << 61) | TAG_FINE | TAG_MID) != 0 || !gone_set.contains(&untag(tag)));
            for k in gone {
                if let Some(Slot::Resident { at, n, lamps, wire_at, wire_n, light_at, light_n, .. }) = self.slots.remove(&k) {
                    let empty = vec![EMPTY; n as usize];
                    renderer.write(&ctx.queue, Target::PropInstances, at as u64 * 48, bytemuck::cast_slice(&empty));
                    self.alloc.free(at, n);
                    // the tile's wire segments and lamp lights.
                    if wire_n > 0 {
                        renderer.write(&ctx.queue, Target::PropWires, wire_at as u64 * 32, bytemuck::cast_slice(&vec![GpuWireSeg::default(); wire_n as usize]));
                        self.wire_alloc.free(wire_at, wire_n);
                    }
                    if light_n > 0 {
                        renderer.write(&ctx.queue, Target::PropLights, light_at as u64 * 64, bytemuck::cast_slice(&vec![GpuLampLight::default(); light_n as usize]));
                        self.light_alloc.free(light_at, light_n);
                    }
                    for l in lamps {
                        self.lamp_src.remove(&l);
                        self.lamp_free.push(l);
                    }
                    self.parked_dirty = true;
                }
            }
        }
        for k in &wanted {
            if !self.slots.contains_key(k) {
                self.slots.insert(*k, Slot::Requested);
                self.outbox.request(*k);
            }
        }
        self.outbox.flush(&self.pending); // try_lock only
        // a new date (a shot's day): the fine cells' grass colour and flowers were cooked
        // on the old one; drop and re-plant them all (the coarse tiles' trees, shrubs and
        // rock do not depend on the date).
        let epoch = veg_epoch();
        let evict_fine = epoch != self.veg_epoch;
        self.veg_epoch = epoch;
        self.pump_ring(RING_FINE, ctx, renderer, uploads, eye, evict_fine);
        self.pump_ring(RING_MID, ctx, renderer, uploads, eye, false);
        if let Some(rx) = &self.tiles_rx {
            while let Ok((mut pack, epoch)) = rx.try_recv() {
                eo_render::hitch::count(eo_render::hitch::Event::Arrival, 1);
                self.stats.tiles_planned += 1;
                self.stats.plan_ms_total += pack.ms;
                self.stats.plan_ms_max = self.stats.plan_ms_max.max(pack.ms);
                let slots = if pack.ring > 0 { &mut self.rings[pack.ring as usize - 1].slots } else { &mut self.slots };
                if !matches!(slots.get(&pack.key), Some(Slot::Requested)) {
                    continue;
                }
                // a fine cell cooked on another date: its re-request is queued.
                if pack.ring == RING_FINE && epoch != self.veg_epoch {
                    continue;
                }
                self.stats.overhead_ms_total += pack.overhead_ms;
                self.stats.overhead_ms_max = self.stats.overhead_ms_max.max(pack.overhead_ms);
                let n = pack.instances.len() as u32;
                let at = if n == 0 { Some(0) } else { self.alloc.alloc(n) };
                let Some(at) = at else {
                    self.errors.push("props: instance arena full".into());
                    if pack.ring > 0 { &mut self.rings[pack.ring as usize - 1].slots } else { &mut self.slots }.remove(&pack.key);
                    continue;
                };
                // wire segments and lamp lights, ranges of their own arenas.
                let wn = pack.wires.len() as u32;
                let ln = pack.lights.len() as u32;
                let (Some(wat), Some(lat)) = (self.wire_alloc.alloc(wn), self.light_alloc.alloc(ln)) else {
                    self.errors.push("props2: wire or light arena full".into());
                    self.alloc.free(at, n);
                    self.slots.remove(&pack.key);
                    continue;
                };
                let mut lamps = vec![];
                for (i, live) in pack.lives.drain(..) {
                    let ch = match self.lamp_free.pop() {
                        Some(c) => c,
                        None if self.lamp_next < LAMP_CAPACITY => {
                            self.lamp_next += 1;
                            self.lamp_next - 1
                        }
                        None => continue,
                    };
                    pack.instances[i as usize].lamp = ch;
                    self.lamp_src.insert(ch, live);
                    lamps.push(ch);
                }
                let mut writes = vec![];
                if n > 0 {
                    let bytes: Vec<u8> = bytemuck::cast_slice(&pack.instances).to_vec();
                    writes.push(Write { target: Target::PropInstances, offset: at as u64 * 48, data: Data::Bytes(bytes) });
                }
                if wn > 0 {
                    writes.push(Write { target: Target::PropWires, offset: wat as u64 * 32, data: Data::Bytes(bytemuck::cast_slice(&pack.wires).to_vec()) });
                }
                if ln > 0 {
                    writes.push(Write { target: Target::PropLights, offset: lat as u64 * 64, data: Data::Bytes(bytemuck::cast_slice(&pack.lights).to_vec()) });
                }
                eo_render::hitch::mark(eo_render::hitch::Mark::PropsTileQueued, 1);
                if !writes.is_empty() {
                    let ring_tag = match pack.ring {
                        RING_FINE => TAG_FINE,
                        RING_MID => TAG_MID,
                        _ => 0,
                    };
                    let job = Job::new(TAG_PROPS | ring_tag | tag_of(pack.key), writes, vec![]);
                    self.stats.bytes_queued += job.bytes();
                    uploads.push(job);
                }
                if pack.ring > 0 { &mut self.rings[pack.ring as usize - 1].slots } else { &mut self.slots }.insert(
                    pack.key,
                    Slot::Resident { at, n, lamps, parked: Arc::new(pack.parked), counts: pack.counts, wire_at: wat, wire_n: wn, light_at: lat, light_n: ln, extra: pack.extra_counts },
                );
                self.parked_dirty = true;
            }
        }
        // No resident tile (e.g. high over the city): no props pass runs at all.
        // the high-water marks are the end of the highest resident range
        // (the allocator's `peak` is a count, which a fragmented arena can exceed).
        let (mut end, mut wend, mut lend) = (0u32, 0u32, 0u32);
        for s in self.slots.values().chain(self.rings.iter().flat_map(|r| r.slots.values())) {
            if let Slot::Resident { at, n, wire_at, wire_n, light_at, light_n, .. } = s {
                if *n > 0 {
                    end = end.max(at + n);
                }
                if *wire_n > 0 {
                    wend = wend.max(wire_at + wire_n);
                }
                if *light_n > 0 {
                    lend = lend.max(light_at + light_n);
                }
            }
        }
        gpu.set_high_water(end);
        gpu.set_wire_high_water(wend);
        // the city lights take these tiles' lamps; the fallback placement stands down in them.
        renderer.look.sky.set_props_tiles(&ctx.queue, self.slots.iter().filter(|(_, s)| matches!(s, Slot::Resident { .. })).map(|(k, _)| *k));
        if self.light_count_written != Some(lend) {
            gpu.set_light_count(&ctx.queue, lend);
            self.light_count_written = Some(lend);
        }
        self.stats.peak_slots = self.stats.peak_slots.max(end);
        self.stats.peak_wire_slots = self.stats.peak_wire_slots.max(wend);
        self.stats.peak_light_slots = self.stats.peak_light_slots.max(lend);
        // Lamp channels on the simulation clock.
        self.clock += dt;
        let t = sim_time.unwrap_or(self.clock);
        gpu.set_time(t); // the wind runs on the simulation clock
        let timing = self.source.as_ref().map(|s| s.timing.clone());
        let len = self.lamp_next as usize;
        self.lamp_words.resize(len, 3);
        for (&ch, src) in &self.lamp_src {
            self.lamp_words[ch as usize] = match src {
                LiveRef::Dark => 3,
                LiveRef::Vehicle { j, axis } => j.aspect_at(*axis, t) as u32 ^ 0, // 0 red, 1 amber, 2 green
                LiveRef::Pedestrian { j, crossed, length } => match (crossed, &timing) {
                    (Some(c), Some(tm)) => match eo_sim::signals::ped_indication(j, *c, t, *length, tm) {
                        3 => 2,
                        2 => (t.fract() < 0.5) as u32,
                        _ => 1,
                    },
                    _ => 0,
                },
            };
        }
        if len > 0 {
            gpu.write_lamps(&ctx.queue, &self.lamp_words);
        }
        self.stats.lamp_channels = self.lamp_src.len();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        self.stats.pump_ms_max = self.stats.pump_ms_max.max(ms);
    }

    /// A vegetation ring's requests, and its cells gone freed: the fine ring
    /// (32 m grass cells within `FINE_LOAD_M` of the eye, dropped past
    /// `FINE_DROP_M`), the mid ring (128 m tiles of hill rock from the props
    /// tiles' reach out to `MID_LOAD_M`, dropped past `MID_DROP_M` or once well
    /// inside the props tiles, which plant the same rocks).
    fn pump_ring(&mut self, ring: u8, ctx: &GpuContext, renderer: &Renderer, uploads: &mut UploadQueue, eye: Vec3, evict_all: bool) {
        let Some(veg) = self.veg.clone() else { return };
        let fine = ring == RING_FINE;
        if (fine && !veg.switches.ground_cover) || (!fine && !hill_rock::props_rock_enabled(&veg)) {
            return;
        }
        let r = &mut self.rings[ring as usize - 1];
        let c = if fine { vegetation::FINE_CELL_M as f32 } else { eo_street::STREET_TILE_M as f32 };
        let (load, drop) = if fine { (vegetation::FINE_LOAD_M, vegetation::FINE_DROP_M) } else { (hill_rock::MID_LOAD_M, hill_rock::MID_DROP_M) };
        let reach = (drop / c).ceil() as i32 + 1;
        let (cx, cz) = ((eye.x / c).floor() as i32, (eye.z / c).floor() as i32);
        let mut wanted: HashSet<Key> = HashSet::new();
        for tz in cz - reach..=cz + reach {
            for tx in cx - reach..=cx + reach {
                let k = (tx, tz);
                let resident = r.slots.contains_key(&k);
                let land = if fine {
                    let (x0, z0) = (tx as f64 * c as f64, tz as f64 * c as f64);
                    [(0.5, 0.5), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().any(|(u, v)| veg.grid.height(x0 + u * c as f64, z0 + v * c as f64).is_some_and(|y| y > eo_water::SEA_LEVEL_Y + 0.3))
                } else {
                    veg.tiles.contains(&k)
                };
                if !land {
                    continue;
                }
                let want = if fine {
                    let (gx, gz) = ((tx as f32 + 0.5) * c, (tz as f32 + 0.5) * c);
                    let gy = veg.grid.height(gx as f64, gz as f64).unwrap_or(eye.y as f64) as f32;
                    let d = fine_distance(k, eye).hypot((eye.y - gy).abs());
                    d <= load || (resident && d <= drop)
                } else {
                    let d = tile_distance_3d(k, eye, |x, z| veg.grid.height(x as f64, z as f64).unwrap_or(0.0) as f32);
                    // from just inside the props tiles' reach (an overlap of identical rocks while they hand over).
                    (d > LOAD_M - 20.0 && d <= load) || (resident && d > LOAD_M - 60.0 && d <= drop)
                };
                if want {
                    wanted.insert(k);
                }
            }
        }
        let tag_bit = if fine { TAG_FINE } else { TAG_MID };
        let gone: Vec<Key> = r.slots.keys().filter(|k| evict_all || !wanted.contains(k)).copied().collect();
        if !gone.is_empty() {
            let gone_set: HashSet<Key> = gone.iter().copied().collect();
            for k in &gone_set {
                r.outbox.cancel(*k);
            }
            uploads.cancel(|tag| tag & TAG_PROPS == 0 || tag & tag_bit == 0 || tag & (eo_render::kit::TAG_KIT | eo_render::street::TAG_STREET | (1 << 60) | (1 << 61)) != 0 || !gone_set.contains(&untag(tag)));
            for k in gone {
                if let Some(Slot::Resident { at, n, .. }) = r.slots.remove(&k) {
                    if n > 0 {
                        renderer.write(&ctx.queue, Target::PropInstances, at as u64 * 48, bytemuck::cast_slice(&vec![EMPTY; n as usize]));
                        self.alloc.free(at, n);
                    }
                }
            }
        }
        for k in &wanted {
            if !r.slots.contains_key(k) {
                r.slots.insert(*k, Slot::Requested);
                r.outbox.request(*k);
            }
        }
        r.outbox.flush(&r.pending); // try_lock only
    }

    /// The parked cars to draw this frame, nearest first within the JS rings'
    /// radii and caps, as agent instances (rebuilt when the camera moved 4 m
    /// or tiles changed).
    pub fn parked_agents(&mut self, lib: Option<&eo_agents::library::Library>, eye: Vec3) -> &[eo_agents::GpuAgent] {
        let Some(lib) = lib else {
            self.parked.clear();
            return &self.parked;
        };
        if !self.on {
            self.parked.clear();
            return &self.parked;
        }
        let moved = self.parked_cam.is_none_or(|c| (c - eye).length() > 4.0);
        if !(moved || self.parked_dirty) {
            return &self.parked;
        }
        self.parked_cam = Some(eye);
        self.parked_dirty = false;
        let Some(world) = &self.world else { return &self.parked };
        let rings = world.js.value("vehicleRings").as_array().cloned().unwrap_or_default();
        let ring: Vec<(f64, usize)> = rings.iter().map(|r| (r["radius"].as_f64().unwrap_or(0.0), r["maxVehicles"].as_u64().unwrap_or(0) as usize)).collect();
        let far = ring.last().map(|r| r.0).unwrap_or(0.0);
        let mut cars: Vec<(f64, &ParkedCar)> = vec![];
        for s in self.slots.values() {
            if let Slot::Resident { parked, .. } = s {
                for c in parked.iter() {
                    let d = ((c.pos[0] - eye.x as f64).powi(2) + (c.pos[2] - eye.z as f64).powi(2)).sqrt();
                    if d <= far {
                        cars.push((d, c));
                    }
                }
            }
        }
        cars.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Ring caps, nearest first: at most the ring's cap inside each radius (cumulative).
        let mut out = vec![];
        let mut taken = 0usize;
        let mut cap = 0usize;
        let mut ri = 0;
        for (d, c) in cars {
            while ri < ring.len() && d > ring[ri].0 {
                ri += 1;
            }
            if ri >= ring.len() {
                break;
            }
            cap = cap.max(ring[..=ri].iter().map(|r| r.1).sum());
            if taken >= cap {
                continue;
            }
            let Some(model) = lib.model(&c.spec).or_else(|| lib.model("sedan")) else { continue };
            out.push(eo_agents::GpuAgent {
                pos: [c.pos[0] as f32, c.pos[1] as f32, c.pos[2] as f32],
                model,
                rot: glam::Quat::from_rotation_y(c.yaw as f32).to_array(),
                color: (c.paint >> 16 & 255) | ((c.paint >> 8 & 255) << 8) | ((c.paint & 255) << 16) | (255 << 24),
                flags: eo_sim::output::FLAG_STOPPED as u32,
                anim: 0.0,
                steer: 0.0,
                speed: 0.0,
                scale: eo_data::groundscale::vehicles() as f32, // the trial's visual vehicle scale, 1.0 unless set
                clip_phase: 0.0,
                clip: 0,
            });
            taken += 1;
        }
        self.parked = out;
        self.stats.parked_submitted = self.parked.len();
        &self.parked
    }

    pub fn resident_counts(&self) -> HashMap<String, usize> {
        let mut by = HashMap::new();
        for s in self.slots.values().chain(self.rings.iter().flat_map(|r| r.slots.values())) {
            if let Slot::Resident { counts, extra, .. } = s {
                for (m, n) in counts {
                    *by.entry(format!("{m:?}")).or_insert(0) += n;
                }
                for (m, n) in extra {
                    *by.entry(m.to_string()).or_insert(0) += n;
                }
            }
        }
        by
    }
}

impl Drop for PropsRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

fn tag_of(k: Key) -> u64 {
    ((k.0 as u32 as u64 & 0xffff) << 16) | (k.1 as u32 as u64 & 0xffff)
}

fn untag(tag: u64) -> Key {
    (((tag >> 16) & 0xffff) as u16 as i16 as i32, (tag & 0xffff) as u16 as i16 as i32)
}

/// 3D distance from the eye to the tile's ground rectangle (y = 0 plane is
/// near enough: the city's ground is within a few hundred metres of it).
/// 3D distance from the eye to the tile's ground at the rectangle's point nearest
/// the eye, less the tallest prop's reach (15 m) vertically.
fn tile_distance_3d(k: Key, eye: Vec3, ground: impl Fn(f32, f32) -> f32) -> f32 {
    let t = eo_street::STREET_TILE_M as f32;
    let (x0, z0) = (k.0 as f32 * t, k.1 as f32 * t);
    let (nx, nz) = (eye.x.clamp(x0, x0 + t), eye.z.clamp(z0, z0 + t));
    let (dx, dz) = (nx - eye.x, nz - eye.z);
    let g = ground(nx, nz);
    let dy = ((eye.y - g).abs() - 15.0).max(0.0);
    (dx * dx + dz * dz + dy * dy).sqrt()
}

/// a tile's box for the view test: its square, from the lowest
/// ground at its corners and centre to 25 m over the highest (poles, trees).
fn tile_box(k: Key, ground: impl Fn(f32, f32) -> f32) -> (Vec3, Vec3) {
    let t = eo_street::STREET_TILE_M as f32;
    let (x0, z0) = (k.0 as f32 * t, k.1 as f32 * t);
    let gs = [ground(x0, z0), ground(x0 + t, z0), ground(x0, z0 + t), ground(x0 + t, z0 + t), ground(x0 + t / 2.0, z0 + t / 2.0)];
    let (lo, hi) = (gs.iter().copied().fold(f32::MAX, f32::min), gs.iter().copied().fold(f32::MIN, f32::max));
    (Vec3::new(x0, lo - 2.0, z0), Vec3::new(x0 + t, hi + 25.0, z0 + t))
}

fn tile_distance(k: Key, eye: Vec3) -> f32 {
    let t = eo_street::STREET_TILE_M as f32;
    let (x0, z0) = (k.0 as f32 * t, k.1 as f32 * t);
    let dx = (x0 - eye.x).max(eye.x - (x0 + t)).max(0.0);
    let dz = (z0 - eye.z).max(eye.z - (z0 + t)).max(0.0);
    let dy = (eye.y - 60.0).abs().max(0.0) * if eye.y > 400.0 { 1.0 } else { 0.0 };
    (dx * dx + dz * dz + dy * dy).sqrt()
}

/// 3D distance from the eye to a fine cell's ground square (the eye's height over the sea level as the vertical).
fn fine_distance(k: Key, eye: Vec3) -> f32 {
    let c = vegetation::FINE_CELL_M as f32;
    let (x0, z0) = (k.0 as f32 * c, k.1 as f32 * c);
    let dx = (x0 - eye.x).max(eye.x - (x0 + c)).max(0.0);
    let dz = (z0 - eye.z).max(eye.z - (z0 + c)).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// `EO_VEG_NEAR="x,z x,z ..."`: print every plant (but grass and tufts)
/// within 30 m of those points with the ground it stands on (a diagnostic:
/// the near view cones of the shot eyes).
fn trace_near(v: &vegetation::VegGround, plants: &[eo_landmarks::vegetation::Plant]) {
    static EYES: std::sync::OnceLock<Vec<(f64, f64)>> = std::sync::OnceLock::new();
    let eyes = EYES.get_or_init(|| {
        std::env::var("EO_VEG_NEAR")
            .map(|s| {
                s.split_whitespace()
                    .filter_map(|p| {
                        let c: Vec<f64> = p.split(',').filter_map(|t| t.parse().ok()).collect();
                        (c.len() == 2).then(|| (c[0], c[1]))
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    for p in plants {
        if matches!(p.kind, eo_landmarks::vegetation::Kind::Grass | eo_landmarks::vegetation::Kind::Tuft | eo_landmarks::vegetation::Kind::Meadow(_)) {
            continue;
        }
        for &(ex, ez) in eyes {
            let d = (p.pos[0] - ex).hypot(p.pos[2] - ez);
            if d <= 30.0 {
                let s = v.site(p.pos[0], p.pos[2]);
                eprintln!("vegetation near ({ex:.1}, {ez:.1}): {:?} at ({:.1}, {:.1}) {d:.1} m, cover {:?} slope {:.0} naip {:?}", p.kind, p.pos[0], p.pos[2], s.and_then(|s| s.cover), s.map(|s| s.slope_deg()).unwrap_or(f64::NAN), v.naip_at(p.pos[0], p.pos[2], 3.0));
            }
        }
    }
}

/// What a fine cell's planting depends on besides the place: the season
/// model's grass colour (`eo_look::season::current`, in 64 steps) and the
/// bloom (`vegetation::bloom` on the sun's day, in 16 steps). A fine cell is
/// cooked again when it changes (`PropsRuntime::pump`).
fn veg_epoch() -> u32 {
    veg_epoch_on(eo_look::sun::day_of_year(), eo_landmarks::vegetation::gg3())
}

fn veg_epoch_on(day: f64, g3: eo_landmarks::vegetation::Gg3) -> u32 {
    let season = (eo_look::season::dryness(day).clamp(0.0, 1.0) * 64.0).round() as u32;
    let bloom = (eo_landmarks::vegetation::bloom(day, g3).clamp(0.0, 1.0) * 16.0).round() as u32;
    season * 17 + bloom
}

/// Plant one fine cell (grass, tufts, wildflowers) as instances (worker thread).
fn cook_fine(veg: Option<&vegetation::VegGround>, key: Key) -> TilePack {
    let t0 = Instant::now();
    let mut instances = vec![];
    let mut extra_counts: HashMap<&'static str, usize> = HashMap::new();
    if let Some(v) = veg {
        let c = vegetation::FINE_CELL_M;
        let (x0, z0) = (key.0 as f64 * c, key.1 as f64 * c);
        let mut plants = vec![];
        eo_landmarks::vegetation::plant_fine_with(x0, z0, x0 + c, z0 + c, &|x, z| v.site(x, z), eo_look::sun::day_of_year(), v.switches, &v.extra(), &mut plants);
        trace_near(v, &plants);
        vegetation::instances(&plants, &mut instances, &mut extra_counts);
        if std::env::var("EO_TRACE_VEG").is_ok() {
            eprintln!("vegetation: fine cell {key:?} {extra_counts:?}");
        }
    }
    TilePack { key, ring: RING_FINE, instances, lives: vec![], parked: vec![], counts: HashMap::new(), ms: t0.elapsed().as_secs_f64() * 1e3, wires: vec![], lights: vec![], overhead_ms: 0.0, extra_counts }
}

/// Plan one tile and build its instances (worker thread).
fn cook(world: &PropsWorld, source: &eo_sim::native::NativeSource, junctions: &Mutex<HashMap<eo_sim::TileKey, Arc<Vec<Arc<SignalJunction>>>>>, key: Key, p2: &Props2Args, facts: &LibFacts) -> TilePack {
    let t0 = Instant::now();
    // a tile with no street tile is the vegetation's alone (land beyond the street world).
    let TilePlacement { mut props, parked, lights } = if world.street.tiles.contains_key(&key) { world.plan_tile(key) } else { TilePlacement { props: vec![], parked: vec![], lights: vec![] } }; // mut
    let veg = facts.veg.as_deref();
    let mut plants: Vec<eo_landmarks::vegetation::Plant> = vec![];
    // the park trees (tile-trees.js `plantTileTrees` over the
    // greens' tree classes) of this street tile, drawn with the street
    // species' ez-tree models (the park species are not in the cooked
    // library): park-oak as street-oak, park-ash as street-ash, park-pine as
    // street-high. Kept a 2 m cell off footprints and the drawn street surface.
    if let Some(cover) = eo_landmarks::ground_cover::installed() {
        let t = eo_street::STREET_TILE_M;
        let (x0, z0) = (key.0 as f64 * t, key.1 as f64 * t);
        let sw = &world.street;
        let sea = eo_water::SEA_LEVEL_Y;
        let species = eo_props::eztree::street_tree_species().ok();
        let variant_of = |name: &str| species.and_then(|s| s.iter().position(|v| v == name)).unwrap_or(0) as u16;
        let (oak, ash, high) = (variant_of("street-oak"), variant_of("street-ash"), variant_of("street-high"));
        let blocked = |x: f64, z: f64| sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || world.ground(x, z) < sea + 0.3;
        for tr in cover.park_trees(x0, z0, x0 + t, z0 + t, &|x, z| world.ground(x, z), &blocked) {
            // a mapped wood's tree as a mature coastal tree (the Presidio forest's species), on the drawn ground.
            if let Some(v) = veg.filter(|v| v.switches.coastal_trees && (cover.tree_class_at(tr.x, tr.z) == 1 || v.in_presidio(tr.x, tr.z))) {
                let site = v.site(tr.x, tr.z);
                // no tree on beach sand or at the water's edge (nothing grows there, `VegGround::site`),
                // and the coastal stand's own lattice plants its ground.
                if site.is_none_or(|s| s.cover == Some(eo_landmarks::vegetation::Cover::Wood)) {
                    continue;
                }
                // round 3: not on rock cover (a cliff over a beach, bare rock), and only under the NAIP image's canopy.
                if !v.tree_ground_ok(tr.x, tr.z) {
                    continue;
                }
                let (sp, sc, hf) = eo_landmarks::vegetation::coastal_tree(tr.x, tr.z, v.shore_m(tr.x, tr.z), eo_landmarks::vegetation::hash((tr.x * 8.0) as i64, (tr.z * 8.0) as i64, 91));
                let k = 0.85 + 0.3 * eo_landmarks::vegetation::hash((tr.x * 8.0) as i64, (tr.z * 8.0) as i64, 92) as f32;
                let tint = eo_landmarks::vegetation::TREE_LEAF_TINT[sp as usize].map(|c| c * k);
                // on the park tree's own ground (the lattice's `y`, its root sink included).
                plants.push(eo_landmarks::vegetation::Plant { kind: eo_landmarks::vegetation::Kind::Tree(sp), pos: [tr.x, tr.y, tr.z], yaw: tr.yaw, scale: [sc, sc * hf, sc], rank: 0.0, tint, grad: [0.0; 2], tip: eo_landmarks::vegetation::TIP_NONE });
                continue;
            }
            let variant = match tr.species {
                "park-oak" => oak,
                "park-ash" => ash,
                _ => high,
            };
            let s = tr.scale as f32;
            props.push(eo_props::Prop { model: Model::Tree, pos: [tr.x, tr.y, tr.z], yaw: tr.yaw, scale: [s, s, s], variant, live: Live::None });
        }
    }
    // the coarse vegetation layer: coastal stands, scrub, hill rock.
    if let Some(v) = veg {
        let t = eo_street::STREET_TILE_M;
        let (x0, z0) = (key.0 as f64 * t, key.1 as f64 * t);
        eo_landmarks::vegetation::plant_coarse_with(x0, z0, x0 + t, z0 + t, &|x, z| v.site(x, z), v.switches, &v.extra(), &mut plants);
        trace_near(v, &plants);
    }
    if std::env::var("EO_TRACE_VEG").is_ok() && !plants.is_empty() {
        let mut by: std::collections::BTreeMap<String, usize> = Default::default();
        for p in &plants {
            *by.entry(format!("{:?}", p.kind)).or_insert(0) += 1;
        }
        eprintln!("vegetation: tile {key:?} {by:?}");
    }
    let mut instances = Vec::with_capacity(props.len() + plants.len());
    let mut lives = vec![];
    let mut counts: HashMap<Model, usize> = HashMap::new();
    let mut extra_counts: HashMap<&'static str, usize> = HashMap::new();
    vegetation::instances(&plants, &mut instances, &mut extra_counts);
    for p in &props {
        // the walk's kinds (hydrants ... signal cabinets) are `--no-furniture`'s.
        if (p.model as u32) >= Model::Hydrant as u32 && !p2.furniture {
            continue;
        }
        *counts.entry(p.model).or_insert(0) += 1;
        let slope = if p.model == Model::TactilePad { (p.variant as f32 - 1000.0) / 1000.0 } else { 0.0 };
        let (model, scale) = match p.model {
            // a species' ez-tree (scale = the JS item's scale, heightScale, scale on the metre mesh), or
            // with `--no-eztree` the card tree at that species' size (crown diameter, height, crown diameter).
            Model::Tree if p2.eztree && facts.species > 0 => (lib2::TREE_BASE + (p.variant as u32 % facts.species as u32), p.scale),
            Model::Tree => {
                let d = facts.tree_dims.get(p.variant as usize % facts.tree_dims.len().max(1)).copied().unwrap_or([9.0, 3.0]);
                let crown = 2.0 * d[1] * p.scale[0];
                (Model::Tree as u32, [crown, d[0] * p.scale[1], crown])
            }
            _ => (p.model as u32, p.scale),
        };
        let i = instances.len() as u32;
        instances.push(GpuPropInstance {
            pos: [p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32],
            yaw: p.yaw as f32,
            scale,
            model,
            variant: p.variant as u32,
            lamp: 0,
            slope,
            pad: 0,
        });
        match p.live {
            Live::None => {}
            Live::Vehicle { node, bearing } => {
                let r = junction_at(source, junctions, node).map(|j| {
                    let axis = axis_for_bearing(&j, bearing);
                    LiveRef::Vehicle { j, axis }
                });
                lives.push((i, r.unwrap_or(LiveRef::Dark)));
            }
            Live::Pedestrian { node, bearing, length_m } => {
                let r = junction_at(source, junctions, node).map(|j| {
                    let crossed = j.crossing_axis(bearing);
                    LiveRef::Pedestrian { length: j.ped_crossing_length(crossed.unwrap_or(0), length_m), j, crossed }
                });
                lives.push((i, r.unwrap_or(LiveRef::Dark)));
            }
        }
    }
    // the district lights' luminaires (kept for the night lighting).
    let lights: Vec<GpuLampLight> = if p2.furniture {
        lights
            .iter()
            .map(|l| GpuLampLight {
                pos: [l.pos[0] as f32, l.pos[1] as f32, l.pos[2] as f32],
                range: l.range_m,
                colour: l.colour_linear,
                intensity: l.intensity,
                angle: l.angle,
                penumbra: l.penumbra,
                decay: l.decay,
                source_lift: l.source_lift_m,
                kind: l.kind as u32,
                pad: [0; 3],
            })
            .collect()
    } else {
        vec![]
    };
    // the street overhead of this tile.
    let t1 = Instant::now();
    let mut wires = vec![];
    if p2.overhead || p2.wires {
        let oh = eo_props::overhead::plan_tile(world, key);
        let mut put = |model: u32, pos: [f64; 3], yaw: f64, scale: [f32; 3], name: &'static str| {
            *extra_counts.entry(name).or_insert(0) += 1;
            instances.push(GpuPropInstance { pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32], yaw: yaw as f32, scale, model, variant: 0, lamp: 0, slope: 0.0, pad: 0 });
        };
        if p2.overhead {
            for pole in &oh.poles {
                match pole.kind {
                    eo_props::overhead::PoleKind::Distribution => put(lib2::OH_DIST_POLE, pole.pos, pole.yaw, [1.0; 3], "overhead distribution pole"),
                    _ => {
                        let wood = pole.material == eo_props::overhead::PoleMaterial::Wood;
                        put(if wood { lib2::OH_POLE_WOOD } else { lib2::OH_POLE_STEEL }, pole.pos, pole.yaw, [1.0, pole.height as f32, 1.0], "overhead span/support pole");
                    }
                }
                if let Some(a) = pole.arm {
                    put(lib2::OH_COBRA_ARM, a.pos, a.yaw, [1.0; 3], "overhead cobra arm");
                }
            }
            for c in &oh.cabinets {
                let n = lib2::CABINET_NOMINAL;
                put(lib2::OH_CABINET, c.pos, c.yaw, [c.size[0] as f32 / n[0], c.size[2] as f32 / n[1], c.size[1] as f32 / n[2]], "overhead signal cabinet");
            }
            for s in &oh.signs {
                if (s.variant as usize) < facts.sign_variants.min(lib2::MAX_SIGN_VARIANTS as usize) {
                    put(lib2::OH_SIGN_BASE + s.variant as u32, s.pos, s.yaw, [s.scale as f32; 3], "overhead sign assembly");
                }
            }
            for c in &oh.corner_signs {
                put(lib2::OH_CORNER_SIGN, c.pos, c.yaw, [1.0; 3], "overhead corner sign");
            }
            for l in &oh.lanterns {
                put(lib2::OH_LANTERN, l.pos, l.yaw, [1.0; 3], "overhead lantern");
            }
        }
        if p2.wires {
            for w in &oh.wires {
                for pair in w.points.windows(2) {
                    wires.push(GpuWireSeg { a: pair[0], radius: w.radius_m, b: pair[1], kind: w.kind as u32 });
                }
            }
            *extra_counts.entry("wire segments").or_insert(0) += wires.len();
            *extra_counts.entry("wires").or_insert(0) += oh.wires.len();
        }
    }
    let overhead_ms = t1.elapsed().as_secs_f64() * 1e3;
    TilePack { key, ring: 0, instances, lives, parked, counts, ms: t0.elapsed().as_secs_f64() * 1e3, wires, lights, overhead_ms, extra_counts }
}


/// `axisForBearing` (ambient-control.js): the axis whose folded bearing is nearest.
fn axis_for_bearing(j: &SignalJunction, bearing: f64) -> usize {
    let fold = |b: f64| b.rem_euclid(std::f64::consts::PI);
    let target = fold(bearing);
    let mut best = 0;
    let mut gap = f64::INFINITY;
    for (i, a) in j.axes.iter().enumerate() {
        let raw = (fold(a.bearing) - target).abs();
        let g = raw.min(std::f64::consts::PI - raw);
        if g < gap {
            gap = g;
            best = i;
        }
    }
    best
}

/// The simulation's controller of the junction at `node` (its tile built by
/// the same `NativeSource` the live simulation streams from).
fn junction_at(source: &eo_sim::native::NativeSource, cache: &Mutex<HashMap<eo_sim::TileKey, Arc<Vec<Arc<SignalJunction>>>>>, node: [f64; 2]) -> Option<Arc<SignalJunction>> {
    let tk = eo_sim::TileKey::of(node[0], node[1]);
    let list = {
        let hit = cache.lock().unwrap().get(&tk).cloned();
        match hit {
            Some(l) => l,
            None => {
                let build = source.build(&[tk]);
                let l: Arc<Vec<Arc<SignalJunction>>> = Arc::new(build.junctions.iter().map(|ji| Arc::new(SignalJunction::build(ji, &source.timing))).collect());
                cache.lock().unwrap().insert(tk, l.clone());
                l
            }
        }
    };
    list.iter()
        .filter(|j| !j.orphan)
        .map(|j| (((j.centre[0] - node[0]).powi(2) + (j.centre[1] - node[1]).powi(2)).sqrt(), j))
        .filter(|(d, _)| *d <= 30.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, j)| j.clone())
}

impl PropsRuntime {
    /// The bench JSON's `props` section.
    pub fn report(&self, gpu_bytes: Option<u64>) -> serde_json::Value {
        let s = &self.stats;
        serde_json::json!({
            "on": self.on,
            "ready_ms_after_start": s.ready_ms,
            "tiles_planned": s.tiles_planned,
            "plan_ms_mean": if s.tiles_planned > 0 { s.plan_ms_total / s.tiles_planned as f64 } else { 0.0 },
            "plan_ms_max": s.plan_ms_max,
            "resident_tiles": self.slots.values().filter(|x| matches!(x, Slot::Resident { .. })).count(),
            "resident_by_model": self.resident_counts(),
            "instance_slots_peak": s.peak_slots,
            "lamp_channels": s.lamp_channels,
            "parked_cars_submitted": s.parked_submitted,
            "instance_bytes_queued": s.bytes_queued,
            "render_thread_pump_ms_max": s.pump_ms_max,
            "gpu_bytes": gpu_bytes,
            "errors": self.errors,
            "props2": format!("{:?}", self.p2),
            "overhead_plan_ms_mean": if s.tiles_planned > 0 { s.overhead_ms_total / s.tiles_planned as f64 } else { 0.0 },
            "overhead_plan_ms_max": s.overhead_ms_max,
            "wire_slots_peak": s.peak_wire_slots,
            "lamp_light_slots_peak": s.peak_light_slots,
        })
    }
}

/// Settle-time helper: the parked cars for this camera into the agents, one frozen agent pump.
pub fn sync_parked(ctx: &GpuContext, renderer: &Renderer, props: &mut PropsRuntime, agents: &mut crate::agents::AgentRuntime, cam: &eo_render::camera::Camera) {
    if !props.on {
        return;
    }
    agents.parked = props.parked_agents(agents.lib.as_deref(), cam.eye).to_vec();
    agents.pump(ctx, renderer, crate::agents::AgentRuntime::focus_of(cam.eye, cam.forward()), 0.0);
}

// ---- one runtime for the modes that hold their GPU state behind shared references (window, agents captures)

static GLOBAL: Mutex<Option<PropsRuntime>> = Mutex::new(None);

pub fn global_start(world: &crate::World, args: &crate::Args) {
    *GLOBAL.lock().unwrap() = Some(PropsRuntime::start(world, args));
}

pub fn global_pump(ctx: &GpuContext, renderer: &Renderer, uploads: &mut UploadQueue, eye: Vec3) {
    if let Some(p) = GLOBAL.lock().unwrap().as_mut() {
        p.pump(ctx, renderer, uploads, eye, None, 0.0);
    }
}

pub fn global_pump_live(ctx: &GpuContext, renderer: &Renderer, uploads: &mut UploadQueue, eye: Vec3, sim_time: f64, dt: f64) {
    if let Some(p) = GLOBAL.lock().unwrap().as_mut() {
        p.pump(ctx, renderer, uploads, eye, Some(sim_time), dt);
    }
}

pub fn global_settled() -> bool {
    GLOBAL.lock().unwrap().as_ref().is_none_or(|p| p.settled())
}

pub fn global_parked(lib: Option<&eo_agents::library::Library>, eye: Vec3) -> Vec<eo_agents::GpuAgent> {
    GLOBAL.lock().unwrap().as_mut().map(|p| p.parked_agents(lib, eye).to_vec()).unwrap_or_default()
}

// ---- props2: matched captures from a pose file (`EO_CAPTURE_POSES`), poses chosen from the data

/// A capture pose as written to / read from the pose file (JSON list).
pub fn pose_json(name: &str, c: &eo_render::camera::Camera, what: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "eye": [c.eye.x, c.eye.y, c.eye.z], "yaw": c.yaw, "pitch": c.pitch, "fov_y_deg": c.fov_y_deg, "near": c.near, "far": c.far, "aspect": c.aspect, "what": what })
}

pub fn read_poses(path: &std::path::Path) -> anyhow::Result<Vec<(String, eo_render::camera::Camera)>> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let f = |x: &serde_json::Value| x.as_f64().unwrap_or(0.0) as f32;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    let e = &p["eye"];
                    (
                        p["name"].as_str().unwrap_or("pose").to_string(),
                        eo_render::camera::Camera { eye: Vec3::new(f(&e[0]), f(&e[1]), f(&e[2])), yaw: f(&p["yaw"]), pitch: f(&p["pitch"]), fov_y_deg: f(&p["fov_y_deg"]), near: f(&p["near"]), far: f(&p["far"]), aspect: f(&p["aspect"]), jitter: [0.0; 2] }, // + jitter
                    )
                })
                .collect()
        })
        .unwrap_or_default())
}

/// the `--capture-props2` poses, chosen from the placement (the same with and
/// without the props2 flags): a signalised junction from 18 m back on its approach, a
/// tree-lined street, a kerb with a hydrant and parked cars, a street with lights and
/// overhead wires, a sign plate close up (each near the SoMa or Inner Sunset oracle
/// window centres), plus the existing props cameras.
pub fn props2_poses(world: &crate::World, aspect: f32, far: f32) -> Vec<(String, eo_render::camera::Camera, String)> {
    let mut out = vec![];
    let Some(street) = world.street.as_ref().map(|s| s.world.clone()) else { return out };
    let Ok(js) = eo_props::JsProps::load() else { return out };
    let Ok(ca) = eo_data::CaStandards::load(&world.paths) else { return out };
    let Ok(std) = eo_props::Standards::load(&ca) else { return out };
    let pw = PropsWorld::new(street.clone(), Arc::new(js), std);
    let _ = pw.install_furniture(&world.city.buildings);
    let plan = &street.plan;
    let ground = |x: f32, z: f32| street.surface_at(x as f64, z as f64, None) as f32;
    let soma = eo_data::groundscale::plan([1600.0f64, -400.0]); // on the scaled plan
    // 1. A signalised junction (3+ approaches, 2+ heads) seen from 18 m back on the middle of an approach.
    let mut nodes: Vec<(f64, usize)> = plan.nodes.iter().enumerate().filter(|(_, n)| n.signalised() && n.approaches.len() >= 3).map(|(i, n)| ((n.position[0] - soma[0]).hypot(n.position[1] - soma[1]), i)).collect();
    nodes.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, ni) in nodes.iter().take(8) {
        let np = eo_props::signals::plan_node_parts(&pw, ni);
        if np.heads.len() < 2 {
            continue;
        }
        let n = &plan.nodes[ni];
        let a = &n.approaches[0];
        let back = a.trim + 18.0;
        let side = [-a.u[1], a.u[0]];
        let off = a.half * 0.35;
        let ex = (n.position[0] + a.u[0] * back + side[0] * off) as f32;
        let ez = (n.position[1] + a.u[1] * back + side[1] * off) as f32;
        let eye = Vec3::new(ex, ground(ex, ez) + 1.7, ez);
        let at = Vec3::new(n.position[0] as f32, ground(n.position[0] as f32, n.position[1] as f32) + 3.5, n.position[1] as f32);
        out.push(("junction".into(), look_at(eye, at, far, aspect), format!("signalised node {} from {back:.0} m back on its approach", n.id)));
        break;
    }
    // Every prop of the tiles round a point (the walk's kinds included).
    let tiles_near = |c: [f64; 2], r: i32| -> Vec<((i32, i32), eo_props::TilePlacement)> {
        let k = eo_street::tile_key(c[0], c[1]);
        let mut v = vec![];
        for dz in -r..=r {
            for dx in -r..=r {
                let key = (k.0 + dx, k.1 + dz);
                if street.tiles.contains_key(&key) {
                    v.push((key, pw.plan_tile(key)));
                }
            }
        }
        v
    };
    let near = tiles_near(soma, 2);
    // 2. A hydrant with parked cars on its kerb within 15 m: from 8 m back along the kerb on the footway.
    let mut best: Option<(f64, eo_props::Prop, eo_props::ParkedCar)> = None;
    for (_, t) in &near {
        for h in t.props.iter().filter(|p| p.model == Model::Hydrant) {
            let d0 = (h.pos[0] - soma[0]).hypot(h.pos[2] - soma[1]);
            for (_, t2) in &near {
                for c in &t2.parked {
                    let d = (c.pos[0] - h.pos[0]).hypot(c.pos[2] - h.pos[2]);
                    if d > 5.0 && d < 15.0 && best.as_ref().is_none_or(|b| d0 < b.0) {
                        best = Some((d0, *h, c.clone()));
                    }
                }
            }
        }
    }
    if let Some((_, h, c)) = best {
        let e = &plan.entries[c.entry as usize];
        let (s, _, _) = eo_props::project(e, h.pos[0], h.pos[2]).unwrap_or((c.station, 0.0, 0.0));
        let dir = if c.station > s { -1.0 } else { 1.0 };
        let f = eo_props::station_at(e, (s + dir * 9.0).clamp(0.0, e.length), false);
        let side = c.side as f64;
        let walk = if side > 0.0 { e.walks[0] } else { e.walks[1] };
        let u = side * (e.half + walk.max(1.0) * 0.55);
        let (ex, ez) = ((f.x + f.nx * u) as f32, (f.z + f.nz * u) as f32);
        let eye = Vec3::new(ex, ground(ex, ez) + 1.6, ez);
        let at = Vec3::new(((h.pos[0] + c.pos[0]) * 0.5) as f32, h.pos[1] as f32 + 0.6, ((h.pos[2] + c.pos[2]) * 0.5) as f32);
        out.push(("kerb-hydrant-parked".into(), look_at(eye, at, far, aspect), format!("hydrant at ({:.1}, {:.1}) and parked {} on ribbon {}", h.pos[0], h.pos[2], c.spec, e.id)));
    }
    // 3. Street lights and overhead wires: the tile near SoMa with the most wire spans and a district light;
    //    on the footway at eye height looking along the ribbon under them.
    let mut best_w: Option<(usize, (i32, i32))> = None;
    for (key, t) in &near {
        let lamps = t.props.iter().filter(|p| matches!(p.model, Model::LampCobra | Model::LampBanner | Model::LampHistoric)).count();
        if lamps == 0 {
            continue;
        }
        let oh = eo_props::overhead::plan_tile(&pw, *key);
        let n = oh.wires.len();
        if best_w.is_none_or(|b| n > b.0) {
            best_w = Some((n, *key));
        }
    }
    if let Some((nw, key)) = best_w {
        let t = pw.plan_tile(key);
        if let Some(l) = t.props.iter().find(|p| matches!(p.model, Model::LampCobra | Model::LampBanner | Model::LampHistoric)) {
            // The lamp's ribbon: nearest entry of the tile.
            let mut be: Option<(f64, usize)> = None;
            if let Some(st) = street.tiles.get(&key) {
                for sp in &st.spans {
                    let e = &plan.entries[sp.0 as usize];
                    if let Some((_, d, _)) = eo_props::project(e, l.pos[0], l.pos[2]) {
                        if be.is_none_or(|b| d < b.0) {
                            be = Some((d, sp.0 as usize));
                        }
                    }
                }
            }
            if let Some((_, ei)) = be {
                let e = &plan.entries[ei];
                let (s, _, lat) = eo_props::project(e, l.pos[0], l.pos[2]).unwrap();
                let dir = if s > e.length / 2.0 { -1.0 } else { 1.0 };
                let f = eo_props::station_at(e, (s - dir * 25.0).clamp(0.0, e.length), false);
                let side = if lat >= 0.0 { 1.0 } else { -1.0 };
                // On the carriageway, a third of the half-width from the lamp's kerb, looking along under the spans.
                let u = side * e.half * 0.35;
                let (ex, ez) = ((f.x + f.nx * u) as f32, (f.z + f.nz * u) as f32);
                let eye = Vec3::new(ex, ground(ex, ez) + 1.7, ez);
                let g = eo_props::station_at(e, (s + dir * 30.0).clamp(0.0, e.length), false);
                let at = Vec3::new((g.x + g.nx * u) as f32, eye.y + 5.0, (g.z + g.nz * u) as f32);
                out.push(("lights-wires".into(), look_at(eye, at, far, aspect), format!("ribbon {} with a {:?} and {nw} overhead wires in tile {key:?}", e.id, l.model)));
            }
        }
    }
    // 4. A sign plate close up: the nearest SFMTA plate to SoMa whose face is in the face atlas
    //    (not a plain colour face), 2.4 m in front of it at its height.
    let mut best_p: Option<(f64, eo_props::Prop)> = None;
    for (_, t) in &near {
        for p in t.props.iter().filter(|p| p.model == Model::SignPlate) {
            let face = pw.js.signs.faces.get(p.variant as usize).map(|s| s.as_str()).unwrap_or("");
            if !eo_props::sign_atlas::SIGN_FACE_NAMES.contains(&face) || p.pos[1] - pw.ground(p.pos[0], p.pos[2]) > 3.5 {
                continue;
            }
            let d = (p.pos[0] - soma[0]).hypot(p.pos[2] - soma[1]);
            if best_p.as_ref().is_none_or(|b| d < b.0) {
                best_p = Some((d, *p));
            }
        }
    }
    if let Some((_, p)) = best_p {
        let n = Vec3::new(p.yaw.sin() as f32, 0.0, p.yaw.cos() as f32);
        let c = Vec3::new(p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32);
        let eye = c + n * 2.4 - Vec3::Y * 0.3;
        let face = pw.js.signs.faces.get(p.variant as usize).cloned().unwrap_or_default();
        out.push(("sign-close".into(), look_at(eye, c, far, aspect), format!("plate '{face}' ({:.2} x {:.2} m) at ({:.1}, {:.1}, {:.1})", p.scale[0], p.scale[1], p.pos[0], p.pos[1], p.pos[2])));
    }
    // 5. The props slice's own cameras (trees near Irving and 8th, the junction, its head, the parked kerb).
    for (name, cam, what) in capture_cameras(world, aspect, far) {
        out.push((format!("props-{name}"), cam, what));
    }
    out
}

// ---- capture cameras (the same with and without --no-props: chosen from the placement, not from what is drawn)

fn look_at(eye: Vec3, at: Vec3, far: f32, aspect: f32) -> eo_render::camera::Camera {
    let d = at - eye;
    let pitch = (d.y / Vec3::new(d.x, 0.0, d.z).length().max(1e-3)).atan();
    eo_render::camera::Camera { far, ..eo_render::camera::Camera::new(eye, d.x.atan2(-d.z), pitch, aspect) }
}

/// A signalised junction near Folsom & 5th, a tree-lined block near Irving
/// Street and 8th Avenue (the oracle windows' centres), a parked kerb near the first.
pub fn capture_cameras(world: &crate::World, aspect: f32, far: f32) -> Vec<(String, eo_render::camera::Camera, String)> {
    let mut out = vec![];
    let Some(street) = world.street.as_ref().map(|s| s.world.clone()) else { return out };
    let Ok(js) = eo_props::JsProps::load() else { return out };
    let Ok(ca) = eo_data::CaStandards::load(&world.paths) else { return out };
    let Ok(std) = eo_props::Standards::load(&ca) else { return out };
    let pw = PropsWorld::new(street.clone(), Arc::new(js), std);
    let plan = &street.plan;
    let ground = |x: f32, z: f32| street.surface_at(x as f64, z as f64, None) as f32;
    // 1. The signalised junction nearest (1600, -400) with heads.
    let soma = eo_data::groundscale::plan([1600.0f64, -400.0]); // on the scaled plan
    let mut nodes: Vec<(f64, usize)> = plan.nodes.iter().enumerate().filter(|(_, n)| n.signalised() && n.approaches.len() >= 3).map(|(i, n)| ((n.position[0] - soma[0]).hypot(n.position[1] - soma[1]), i)).collect();
    nodes.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, ni) in nodes.iter().take(6) {
        let np = eo_props::signals::plan_node_parts(&pw, ni);
        if np.heads.len() < 2 {
            continue;
        }
        let n = &plan.nodes[ni];
        let a = &n.approaches[0];
        let e = &plan.entries[a.entry];
        let back = a.trim + 16.0;
        let side = [-a.u[1], a.u[0]];
        let off = a.half + 0.5 * if a.at_start { e.walks[0] } else { e.walks[1] };
        let ex = (n.position[0] + a.u[0] * back + side[0] * off) as f32;
        let ez = (n.position[1] + a.u[1] * back + side[1] * off) as f32;
        let eye = Vec3::new(ex, ground(ex, ez) + 1.7, ez);
        let at = Vec3::new(n.position[0] as f32, ground(n.position[0] as f32, n.position[1] as f32) + 4.0, n.position[1] as f32);
        out.push(("junction".into(), look_at(eye, at, far, aspect), format!("signalised node {} ({} heads, {} poles, {} pedestrian heads planned)", n.id, np.heads.len(), np.poles.len(), np.ped_heads.len())));
        // The first vehicle head from 7 m in front of it, a little below.
        let h = &np.heads[0];
        let f = Vec3::new(h.yaw.sin() as f32, 0.0, h.yaw.cos() as f32);
        let hp = Vec3::new(h.x as f32, h.y as f32, h.z as f32);
        let eye = hp + f * 7.0 - Vec3::Y * 1.5;
        out.push(("signal-head".into(), look_at(eye, hp, far, aspect), format!("vehicle head at ({:.1}, {:.1}, {:.1}) of node {}", h.x, h.y, h.z, n.id)));
        break;
    }
    // 2. Trees: the ribbon near (-4085, 1540) carrying the most list trees.
    let irving = eo_data::groundscale::plan([-4085.0f64, 1540.0]);
    let mut best: Option<(usize, usize)> = None;
    for (ei, e) in plan.entries.iter().enumerate() {
        let Some(p) = e.points.first() else { continue };
        if (p[0] - irving[0]).hypot(p[1] - irving[1]) > 250.0 * eo_data::groundscale::scale() {
            continue;
        }
        let n = eo_props::trees::plan_entry(&pw, ei).len();
        if best.is_none_or(|b| n > b.1) {
            best = Some((ei, n));
        }
    }
    if let Some((ei, n)) = best {
        let e = &plan.entries[ei];
        let trees = eo_props::trees::plan_entry(&pw, ei);
        if let Some(t) = trees.first() {
            let (s, _, lat) = eo_props::project(e, t.pos[0], t.pos[2]).unwrap_or((e.length / 2.0, 0.0, e.half));
            let f = eo_props::station_at(e, (s - 10.0).max(0.0), false);
            let side = if lat >= 0.0 { 1.0 } else { -1.0 };
            let u = side * (e.half - 1.5);
            let (ex, ez) = ((f.x + f.nx * u) as f32, (f.z + f.nz * u) as f32);
            let eye = Vec3::new(ex, ground(ex, ez) + 1.7, ez);
            let g = eo_props::station_at(e, (s + 30.0).min(e.length), false);
            let (ax, az) = ((g.x + g.nx * side * (e.half + 1.0)) as f32, (g.z + g.nz * side * (e.half + 1.0)) as f32);
            let at = Vec3::new(ax, ground(ax, az) + 3.0, az);
            out.push(("trees".into(), look_at(eye, at, far, aspect), format!("ribbon {} with {n} list trees planned", e.id)));
        }
    }
    // 3. A parked kerb near (1600, -400).
    let mut best: Option<(f64, usize)> = None;
    for (ei, e) in plan.entries.iter().enumerate() {
        let Some(p) = e.points.first() else { continue };
        let d = (p[0] - soma[0]).hypot(p[1] - soma[1]);
        if d > 300.0 || best.is_some_and(|b| d >= b.0) {
            continue;
        }
        if eo_props::parking::plan_entry(&pw, ei).len() >= 4 {
            best = Some((d, ei));
        }
    }
    if let Some((_, ei)) = best {
        let e = &plan.entries[ei];
        let cars = eo_props::parking::plan_entry(&pw, ei);
        let c = &cars[0];
        let side = c.side as f64;
        let f = eo_props::station_at(e, (c.station - 9.0).max(0.0), false);
        let walk = if side > 0.0 { e.walks[0] } else { e.walks[1] };
        let u = side * (e.half + walk.max(1.0) * 0.5);
        let (ex, ez) = ((f.x + f.nx * u) as f32, (f.z + f.nz * u) as f32);
        let eye = Vec3::new(ex, ground(ex, ez) + 1.7, ez);
        let g = eo_props::station_at(e, (c.station + 12.0).min(e.length), false);
        let (ax, az) = ((g.x + g.nx * side * (e.half - 1.0)) as f32, (g.z + g.nz * side * (e.half - 1.0)) as f32);
        let at = Vec3::new(ax, ground(ax, az) + 0.8, az);
        out.push(("parked".into(), look_at(eye, at, far, aspect), format!("ribbon {} kerb {} with {} cars planned", e.id, c.side, cars.len())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_date_is_a_new_vegetation_epoch() {
        let g3 = eo_landmarks::vegetation::Gg3::ALL;
        // the headland photograph's 13 April (green, in bloom) and 22 September (cured, none).
        assert_ne!(veg_epoch_on(103.0, g3), veg_epoch_on(265.0, g3));
        assert_eq!(veg_epoch_on(103.0, g3), veg_epoch_on(103.0, g3));
        // the bloom alone (same season step) changes it too: 1 June, the window just shut.
        assert_ne!(veg_epoch_on(140.0, g3), veg_epoch_on(160.0, g3));
    }
}
