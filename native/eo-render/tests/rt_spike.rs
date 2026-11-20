//! Stage R0: can this machine ray trace through wgpu? A feasibility test on
//! its own device (it touches no renderer state): it asks the adapter for
//! `Features::EXPERIMENTAL_RAY_QUERY`, builds one bottom-level acceleration
//! structure (BLAS) per cooked kit tile around downtown (plus one for the
//! grade-separated structures there), one top-level structure (TLAS) over
//! several hundred instances, and casts inline ray queries from a compute
//! shader and from a fragment shader (`rt_spike.wgsl`) over a 1920 x 1080
//! grid: primary rays, then hemisphere rays from each primary hit. It times
//! everything with GPU timestamps and writes the numbers and three PNGs of
//! its own render to `tmp/plan-run/R0/` (`EO_RT_OUT` overrides).
//!
//! Skipped, with a message, when the adapter lacks ray queries or the kit
//! tile cache is absent. `EO_RT_KIT_DIR` names a `cache/kit-tiles/<key>`
//! folder (default: the newest under `native/cache/kit-tiles`);
//! `EO_RT_NO_STRUCTURES=1` skips the structures (they need the street world,
//! which takes a while to build).
//!
//! API recipe (wgpu 30.0.2; read from the registry sources):
//! - `wgpu-types/src/tokens.rs`: `ExperimentalFeatures::enabled()` (unsafe)
//!   must be passed in `DeviceDescriptor::experimental_features` to request
//!   any `EXPERIMENTAL_*` feature.
//! - `wgpu-hal/src/metal/adapter.rs`: Metal sets `EXPERIMENTAL_RAY_QUERY`
//!   only when the device supports ray tracing from compute and render
//!   (macOS 15+); the limits `max_acceleration_structures_per_shader_stage`
//!   is 0 in `Limits::default()`, so the adapter's limits are requested.
//! - `wgpu/src/api/{blas,tlas,device,command_encoder}.rs`: `create_blas`,
//!   `create_tlas`, `TlasInstance::new`, `build_acceleration_structures`.
//! - `wgpu-core/src/command/ray_tracing.rs`: only full builds exist
//!   (`PreferUpdate` logs a warning and rebuilds); the TLAS instances are
//!   serialized on the CPU and copied through a staging buffer per build.
//! - `wgpu-types/src/ray_tracing.rs`: geometry must be `OPAQUE` to be hit
//!   (no candidate intersections in naga yet).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eo_render::camera::Camera;
use eo_render::kit::Pack;
use eo_render::scene::Vertex;
use eo_render::wgpu;
use glam::Vec3;
use wgpu::util::DeviceExt;

const W: u32 = 1920;
const H: u32 = 1080;
/// The kit tiles loaded (128 m tiles, `kit::KIT_TILE_M`): x 9..=14,
/// z -16..=-9, the dense downtown blocks of the cache (the largest packs).
const TILE_X: std::ops::RangeInclusive<i32> = 9..=14;
const TILE_Z: std::ops::RangeInclusive<i32> = -16..=-9;
const REPEATS: usize = 5;
const STRIDE: u64 = std::mem::size_of::<Vertex>() as u64;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn out_dir() -> PathBuf {
    std::env::var_os("EO_RT_OUT").map(PathBuf::from).unwrap_or_else(|| repo().join("tmp/plan-run/R0"))
}

/// One BLAS's geometry in the shared buffers.
#[derive(Clone, Debug)]
struct Geom {
    name: String,
    first_vertex: u32,
    vertex_count: u32,
    first_index: u32,
    index_count: u32,
}

struct Scene {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    geoms: Vec<Geom>,
    /// Building boxes (for placing the street camera).
    boxes: Vec<(Vec3, Vec3)>,
    min: Vec3,
    max: Vec3,
    kit_dir: PathBuf,
    load_s: f64,
    structures_s: Option<f64>,
}

impl Scene {
    fn push(&mut self, name: String, verts: &[Vertex], local_indices: &[u32]) {
        if local_indices.len() < 3 {
            return;
        }
        let g = Geom {
            name,
            first_vertex: self.vertices.len() as u32,
            vertex_count: verts.len() as u32,
            first_index: self.indices.len() as u32,
            index_count: (local_indices.len() / 3 * 3) as u32,
        };
        for v in verts {
            let p = Vec3::from(v.pos);
            self.min = self.min.min(p);
            self.max = self.max.max(p);
        }
        self.vertices.extend_from_slice(verts);
        self.indices.extend_from_slice(&local_indices[..g.index_count as usize]);
        self.geoms.push(g);
    }
    fn triangles(&self) -> u64 {
        self.geoms.iter().map(|g| g.index_count as u64 / 3).sum()
    }
}

fn kit_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("EO_RT_KIT_DIR") {
        return Some(PathBuf::from(d));
    }
    let root = repo().join("native/cache/kit-tiles");
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    // the newest folder whose downtown pack decodes (the cache version matches).
    dirs.into_iter().map(|d| d.1).find(|d| std::fs::read(d.join("12_-12_r0.eokt")).ok().is_some_and(|b| Pack::decode(&b).is_ok()))
}

fn load_scene() -> Option<Scene> {
    let dir = kit_dir()?;
    let t0 = Instant::now();
    let files: Vec<PathBuf> = TILE_X.flat_map(|x| TILE_Z.map(move |z| (x, z))).map(|(x, z)| dir.join(format!("{x}_{z}_r0.eokt"))).filter(|p| p.exists()).collect();
    let packs: Vec<(String, Pack)> = std::thread::scope(|s| {
        let chunks: Vec<_> = files.chunks(files.len().div_ceil(8).max(1)).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|c| {
                s.spawn(move || {
                    c.iter()
                        .filter_map(|p| {
                            let pack = Pack::decode(&std::fs::read(p).ok()?).ok()?;
                            Some((p.file_stem()?.to_string_lossy().into_owned(), pack))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    if packs.is_empty() {
        return None;
    }
    let mut scene = Scene { vertices: vec![], indices: vec![], geoms: vec![], boxes: vec![], min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN), kit_dir: dir, load_s: 0.0, structures_s: None };
    for (name, pack) in &packs {
        // a pack's indices are local to each building's first vertex: make
        // them local to the tile's first vertex (the BLAS's vertex range).
        let mut local = Vec::with_capacity(pack.indices.len());
        for b in &pack.buildings {
            let r = b.index_start as usize..(b.index_start + b.index_count) as usize;
            local.extend(pack.indices[r].iter().map(|i| i + b.vertex_start));
            scene.boxes.push((Vec3::from(b.min), Vec3::from(b.max)));
        }
        scene.push(format!("kit {name}"), &pack.vertices, &local);
    }
    scene.load_s = t0.elapsed().as_secs_f64();
    if std::env::var_os("EO_RT_NO_STRUCTURES").is_none() {
        let t1 = Instant::now();
        match structures(scene.min, scene.max) {
            Ok(Some((v, i))) => {
                scene.push("structures".into(), &v, &i);
                scene.structures_s = Some(t1.elapsed().as_secs_f64());
            }
            Ok(None) => println!("structures: none inside the kit tiles' box"),
            Err(e) => println!("structures: not loaded ({e})"),
        }
    }
    Some(scene)
}

/// The grade-separated structure meshes (lod 0) over the kit tiles' box, as
/// one mesh. Built as eo-structures' real-data test builds the world.
fn structures(min: Vec3, max: Vec3) -> anyhow::Result<Option<(Vec<Vertex>, Vec<u32>)>> {
    use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths)?);
    let o = StreetOptions::load()?;
    let sc = StreetCity::load()?;
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let streets = StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
    let ca = eo_data::CaStandards::load(&paths)?;
    let world = eo_structures::StructureWorld::build(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), eo_structures::StructureOptions::default())?;
    let (mut v, mut i) = (Vec::new(), Vec::new());
    for key in world.tile_keys() {
        let Some(m) = world.render_tile_shared(key, 0) else { continue };
        let inside = m.positions.iter().any(|p| p[0] >= min.x && p[0] <= max.x && p[2] >= min.z && p[2] <= max.z);
        if !inside {
            continue;
        }
        let base = v.len() as u32;
        v.extend(m.positions.iter().map(|p| Vertex { pos: *p, normal: 0, color: 0 }));
        i.extend(m.indices.iter().map(|x| x + base));
    }
    Ok((!i.is_empty()).then_some((v, i)))
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    timestamps: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer)>,
    period_ns: f64,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Gpu {
    fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }

    /// Records `f` (which writes timestamps 0 and 1 when given the query
    /// set), submits, waits. Returns (GPU ms between the two timestamps,
    /// wall ms from submit to completion).
    fn timed(&self, f: impl FnOnce(&mut wgpu::CommandEncoder, Option<&wgpu::QuerySet>)) -> (Option<f64>, f64) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        f(&mut enc, self.timestamps.as_ref().map(|t| &t.0));
        if let Some((qs, resolve, read)) = &self.timestamps {
            enc.resolve_query_set(qs, 0..2, resolve, 0);
            enc.copy_buffer_to_buffer(resolve, 0, read, 0, 16);
        }
        let cb = enc.finish();
        let t0 = Instant::now();
        self.queue.submit([cb]);
        self.wait();
        let wall = t0.elapsed().as_secs_f64() * 1e3;
        let gpu = self.timestamps.as_ref().map(|(_, _, read)| {
            let b = self.read(read);
            let t: [u64; 2] = bytemuck::pod_read_unaligned(&b[..16]);
            t[1].saturating_sub(t[0]) as f64 * self.period_ns * 1e-6
        });
        (gpu, wall)
    }

    fn read(&self, buf: &wgpu::Buffer) -> Vec<u8> {
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.wait();
        let out = slice.get_mapped_range().expect("mapped").to_vec();
        buf.unmap();
        out
    }

    fn errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.errors.lock().unwrap())
    }
}

/// An acceleration-structure build between two empty timed compute passes:
/// the end of the first and the start of the second bracket it.
fn timed_build<'a>(gpu: &Gpu, blas: &'a [wgpu::BlasBuildEntry<'a>], tlas: Option<&'a wgpu::Tlas>) -> (Option<f64>, f64) {
    gpu.timed(|enc, q| {
        if q.is_some() {
            drop(enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("rt-before"), timestamp_writes: q.map(|q| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(0) }) }));
        }
        enc.build_acceleration_structures(blas.iter(), tlas.into_iter());
        if q.is_some() {
            drop(enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("rt-after"), timestamp_writes: q.map(|q| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(1), end_of_pass_write_index: None }) }));
        }
    })
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() { f64::NAN } else { v[v.len() / 2] }
}

fn blas_size(g: &Geom) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: g.vertex_count,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(g.index_count),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    }
}

/// Bytes Metal reports for each structure (size, build scratch), through
/// wgpu-hal's `get_acceleration_structure_build_sizes` (wgpu does not
/// expose them).
#[cfg(target_os = "macos")]
fn metal_sizes(device: &wgpu::Device, vb: &wgpu::Buffer, ib: &wgpu::Buffer, geoms: &[Geom], flags: wgpu::AccelerationStructureFlags, tlas_counts: &[u32]) -> Option<(Vec<(u64, u64)>, Vec<(u64, u64)>)> {
    use wgpu::hal;
    use wgpu::hal::Device as _;
    unsafe {
        let d = device.as_hal::<hal::api::Metal>()?;
        let hvb = vb.as_hal::<hal::api::Metal>()?;
        let hib = ib.as_hal::<hal::api::Metal>()?;
        let blas = geoms
            .iter()
            .map(|g| {
                let entries = hal::AccelerationStructureEntries::Triangles(vec![hal::AccelerationStructureTriangles {
                    vertex_buffer: Some(&*hvb),
                    vertex_format: wgpu::VertexFormat::Float32x3,
                    first_vertex: g.first_vertex,
                    vertex_count: g.vertex_count,
                    vertex_stride: STRIDE,
                    indices: Some(hal::AccelerationStructureTriangleIndices { format: wgpu::IndexFormat::Uint32, buffer: Some(&*hib), offset: g.first_index * 4, count: g.index_count }),
                    transform: None,
                    flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
                }]);
                let s = d.get_acceleration_structure_build_sizes(&hal::GetAccelerationStructureBuildSizesDescriptor { entries: &entries, flags });
                (s.acceleration_structure_size, s.build_scratch_size)
            })
            .collect();
        let tlas = tlas_counts
            .iter()
            .map(|&n| {
                // the instance buffer is not read for the size query.
                let entries = hal::AccelerationStructureEntries::Instances(hal::AccelerationStructureInstances { buffer: Some(&*hvb), offset: 0, count: n });
                let s = d.get_acceleration_structure_build_sizes(&hal::GetAccelerationStructureBuildSizesDescriptor { entries: &entries, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE });
                (s.acceleration_structure_size, s.build_scratch_size)
            })
            .collect();
        Some((blas, tlas))
    }
}
#[cfg(not(target_os = "macos"))]
fn metal_sizes(_: &wgpu::Device, _: &wgpu::Buffer, _: &wgpu::Buffer, _: &[Geom], _: wgpu::AccelerationStructureFlags, _: &[u32]) -> Option<(Vec<(u64, u64)>, Vec<(u64, u64)>)> {
    None
}

fn translate(t: Vec3) -> [f32; 12] {
    [1.0, 0.0, 0.0, t.x, 0.0, 1.0, 0.0, t.y, 0.0, 0.0, 1.0, t.z]
}

/// A TLAS over `n` instances cycling through the BLASes, each copy of the
/// block on a grid `step` apart (block 0 at its real place).
fn make_tlas(device: &wgpu::Device, blases: &[wgpu::Blas], n: u32, step: Vec3, per_row: u32) -> wgpu::Tlas {
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("rt-tlas"), max_instances: n, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
    let nb = blases.len() as u32;
    for i in 0..n {
        let b = i % nb;
        let copy = i / nb;
        // copies spiral out from the real block: 0, then a square grid.
        let (cx, cz) = ((copy % per_row) as i32 - (per_row / 2) as i32, (copy / per_row) as i32 - (per_row / 2) as i32);
        let (cx, cz) = if copy == 0 { (0, 0) } else if (cx, cz) == (0, 0) { (per_row as i32 / 2 + 1, 0) } else { (cx, cz) };
        tlas[i as usize] = Some(wgpu::TlasInstance::new(&blases[b as usize], translate(Vec3::new(cx as f32 * step.x, 0.0, cz as f32 * step.z)), b, 0xff));
    }
    tlas
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CamU {
    origin: [f32; 3],
    tan_half: f32,
    right: [f32; 3],
    aspect: f32,
    up: [f32; 3],
    frame: u32,
    fwd: [f32; 3],
    rays: u32,
    size: [u32; 2],
    tmax_bounce: f32,
    bounce_flags: u32,
}

fn cam_u(c: &Camera, frame: u32, rays: u32, tmax_bounce: f32, bounce_flags: u32) -> CamU {
    let f = c.forward();
    let r = c.right();
    let u = r.cross(f).normalize();
    CamU { origin: c.eye.to_array(), tan_half: (c.fov_y_deg.to_radians() * 0.5).tan(), right: r.to_array(), aspect: W as f32 / H as f32, up: u.to_array(), frame, fwd: f.to_array(), rays, size: [W, H], tmax_bounce, bounce_flags }
}

/// Distance along a ray to the first building box (CPU; for the camera).
fn clearance(boxes: &[(Vec3, Vec3)], o: Vec3, d: Vec3, far: f32) -> f32 {
    let inv = d.recip();
    let mut best = far;
    for (mn, mx) in boxes {
        let t0 = (*mn - o) * inv;
        let t1 = (*mx - o) * inv;
        let tn = t0.min(t1).max_element();
        let tf = t0.max(t1).min_element();
        if tf >= tn.max(0.0) && tn < best {
            best = tn.max(0.0);
        }
    }
    best
}

/// A street-level camera: the spot in the middle of the block, outside every
/// building box, with the longest clear view along one of 16 headings.
fn street_camera(scene: &Scene, ground: &dyn Fn(f32, f32) -> f32) -> Camera {
    let c = (scene.min + scene.max) * 0.5;
    let half = ((scene.max - scene.min) * 0.25).min(Vec3::splat(200.0));
    let mut best = (0.0f32, c, 0.0f32);
    let mut z = c.z - half.z;
    while z <= c.z + half.z {
        let mut x = c.x - half.x;
        while x <= c.x + half.x {
            let p = Vec3::new(x, ground(x, z) + 1.7, z);
            let inside = scene.boxes.iter().any(|(mn, mx)| p.cmpge(*mn - 0.5).all() && p.cmple(*mx + 0.5).all());
            if !inside {
                for k in 0..16 {
                    let yaw = k as f32 * std::f32::consts::TAU / 16.0;
                    let cam = Camera::new(p, yaw, 0.0, 1.0);
                    let d = clearance(&scene.boxes, p, cam.forward(), 600.0);
                    if d > best.0 {
                        best = (d, p, yaw);
                    }
                }
            }
            x += 8.0;
        }
        z += 8.0;
    }
    println!("street camera: eye ({:.1}, {:.1}, {:.1}) yaw {:.2} rad, {:.0} m clear ahead", best.1.x, best.1.y, best.1.z, best.2, best.0);
    Camera::new(best.1, best.2, 0.02, W as f32 / H as f32)
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("png file"));
    let mut e = png::Encoder::new(file, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().expect("png header").write_image_data(rgba).expect("png data");
    println!("wrote {}", path.display());
}

fn to_rgba(texels: &[[f32; 4]], f: impl Fn([f32; 4]) -> [f32; 3]) -> Vec<u8> {
    texels.iter().flat_map(|t| {
        let c = f(*t);
        [(c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8, 255]
    }).collect()
}

#[test]
fn ray_queries_on_real_city_geometry() {
    let uptime = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("uptime: {uptime}");
    let instance = eo_render::GpuContext::create_instance(&eo_render::GpuOptions::default());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: false, compatible_surface: None, apply_limit_buckets: false })) else {
        println!("SKIP: no GPU adapter");
        return;
    };
    let info = adapter.get_info();
    let af = adapter.features();
    println!("adapter: {} ({:?}, driver {} {})", info.name, info.backend, info.driver, info.driver_info);
    println!("adapter ray features: EXPERIMENTAL_RAY_QUERY {}, EXPERIMENTAL_RAY_HIT_VERTEX_RETURN {}, EXPERIMENTAL_RAY_TRACING_PIPELINES {}, TIMESTAMP_QUERY {}",
        af.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY), af.contains(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN), af.contains(wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES), af.contains(wgpu::Features::TIMESTAMP_QUERY));
    if !af.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        println!("SKIP: adapter {} lacks Features::EXPERIMENTAL_RAY_QUERY", info.name);
        return;
    }
    let lim = adapter.limits();
    println!("limits: max_blas_primitive_count {}, max_blas_geometry_count {}, max_tlas_instance_count {}, max_acceleration_structures_per_shader_stage {}, max_storage_buffer_binding_size {}, max_buffer_size {}",
        lim.max_blas_primitive_count, lim.max_blas_geometry_count, lim.max_tlas_instance_count, lim.max_acceleration_structures_per_shader_stage, lim.max_storage_buffer_binding_size, lim.max_buffer_size);

    let Some(scene) = load_scene() else {
        println!("SKIP: no kit tile cache (native/cache/kit-tiles, or EO_RT_KIT_DIR)");
        return;
    };
    let tris = scene.triangles();
    println!("scene: {} BLAS geometries from {}, {} vertices, {} triangles (kit load {:.2} s, structures {:?} s), box {:?}..{:?}",
        scene.geoms.len(), scene.kit_dir.file_name().unwrap().to_string_lossy(), scene.vertices.len(), tris, scene.load_s, scene.structures_s.map(|s| (s * 100.0).round() / 100.0), scene.min, scene.max);

    let mut feats = wgpu::Features::EXPERIMENTAL_RAY_QUERY;
    if af.contains(wgpu::Features::TIMESTAMP_QUERY) {
        feats |= wgpu::Features::TIMESTAMP_QUERY;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rt-spike"),
        required_features: feats,
        required_limits: lim.clone(),
        // SAFETY: the experimental ray-query API is opted into for this test only.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .expect("a device with EXPERIMENTAL_RAY_QUERY");
    let errors = Arc::new(Mutex::new(Vec::new()));
    {
        let errors = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            eprintln!("wgpu error: {e}");
            errors.lock().unwrap().push(e.to_string());
        }));
    }
    let timestamps = feats.contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
        let qs = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("rt-ts"), ty: wgpu::QueryType::Timestamp, count: 2 });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-ts-resolve"), size: 16, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-ts-read"), size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        (qs, resolve, read)
    });
    let period_ns = queue.get_timestamp_period() as f64;
    let gpu = Gpu { device, queue, timestamps, period_ns, errors };
    let device = &gpu.device;

    // geometry buffers: the kit's own 20-byte vertices (position first), read
    // by the BLAS builds and by the shaders' normal fetch.
    let input = wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE;
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-vertices"), contents: bytemuck::cast_slice(&scene.vertices), usage: input });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-indices"), contents: bytemuck::cast_slice(&scene.indices), usage: input });
    let geo_table: Vec<[u32; 4]> = scene.geoms.iter().map(|g| [g.first_vertex, g.first_index, 0, 0]).collect();
    let gb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rt-geoms"), contents: bytemuck::cast_slice(&geo_table), usage: wgpu::BufferUsages::STORAGE });
    gpu.wait();

    // --- BLAS builds -------------------------------------------------------
    let sizes: Vec<_> = scene.geoms.iter().map(blas_size).collect();
    let make_blases = |flags: wgpu::AccelerationStructureFlags| -> Vec<wgpu::Blas> {
        scene.geoms.iter().zip(&sizes).map(|(g, s)| device.create_blas(&wgpu::CreateBlasDescriptor { label: Some(&g.name), flags, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![s.clone()] })).collect()
    };
    let build_all = |blases: &[wgpu::Blas]| -> (Option<f64>, f64) {
        let entries: Vec<wgpu::BlasBuildEntry> = blases
            .iter()
            .zip(&sizes)
            .zip(&scene.geoms)
            .map(|((b, s), g)| wgpu::BlasBuildEntry {
                blas: b,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry { size: s, vertex_buffer: &vb, first_vertex: g.first_vertex, vertex_stride: STRIDE, index_buffer: Some(&ib), first_index: Some(g.first_index), transform_buffer: None, transform_buffer_offset: None }]),
            })
            .collect();
        timed_build(&gpu, &entries, None)
    };
    let mtris = tris as f64 / 1e6;
    let mut blas_report = serde_json::Map::new();
    for (label, flags) in [("fast_build", wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD), ("fast_trace", wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE)] {
        let mut g = vec![];
        let mut w = vec![];
        for _ in 0..3 {
            let b = make_blases(flags);
            let (gt, wt) = build_all(&b);
            g.extend(gt);
            w.push(wt);
        }
        let (gm, wm) = (median(g), median(w));
        println!("BLAS build {label}: {} BLASes, {:.2} M triangles: GPU {:.1} ms ({:.1} ms per M triangles), wall {:.1} ms (incl. scratch alloc)", scene.geoms.len(), mtris, gm, gm / mtris, wm);
        let mem = metal_sizes(device, &vb, &ib, &scene.geoms, flags, &[]).map(|(b, _)| (b.iter().map(|s| s.0).sum::<u64>(), b.iter().map(|s| s.1).sum::<u64>(), b.iter().map(|s| s.1).max().unwrap_or(0)));
        if let Some((sz, scratch, scratch_max)) = mem {
            println!("  BLAS memory {label}: {:.1} MB structures ({:.1} B per triangle), build scratch {:.1} MB summed / {:.1} MB largest", sz as f64 / 1e6, sz as f64 / tris as f64, scratch as f64 / 1e6, scratch_max as f64 / 1e6);
        }
        blas_report.insert(label.into(), serde_json::json!({ "gpu_ms": gm, "wall_ms": wm, "gpu_ms_per_mtri": gm / mtris, "bytes": mem.map(|m| m.0), "scratch_bytes_sum": mem.map(|m| m.1) }));
    }
    // the largest single tile on its own (latency of one tile's build).
    let big = (0..scene.geoms.len()).max_by_key(|&i| scene.geoms[i].index_count).unwrap();
    let single = {
        let mut gs = vec![];
        for _ in 0..3 {
            let b = device.create_blas(&wgpu::CreateBlasDescriptor { label: Some("rt-single"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build }, wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![sizes[big].clone()] });
            let g = &scene.geoms[big];
            let e = [wgpu::BlasBuildEntry { blas: &b, geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry { size: &sizes[big], vertex_buffer: &vb, first_vertex: g.first_vertex, vertex_stride: STRIDE, index_buffer: Some(&ib), first_index: Some(g.first_index), transform_buffer: None, transform_buffer_offset: None }]) }];
            gs.push(timed_build(&gpu, &e, None));
        }
        let gm = median(gs.iter().filter_map(|x| x.0).collect());
        let wm = median(gs.iter().map(|x| x.1).collect());
        println!("BLAS build one tile ({}, {} triangles): GPU {:.2} ms, wall {:.2} ms", scene.geoms[big].name, scene.geoms[big].index_count / 3, gm, wm);
        serde_json::json!({ "tile": scene.geoms[big].name, "triangles": scene.geoms[big].index_count / 3, "gpu_ms": gm, "wall_ms": wm })
    };
    let blases = make_blases(wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE);
    let (_, w) = build_all(&blases);
    println!("BLAS set for tracing built (wall {w:.1} ms); errors so far: {:?}", gpu.errors());

    // --- TLAS builds -------------------------------------------------------
    let ext = scene.max - scene.min;
    let step = Vec3::new(ext.x.ceil() + 50.0, 0.0, ext.z.ceil() + 50.0);
    let counts = [100u32, 1_000, 10_000];
    let tlas_mem = metal_sizes(device, &vb, &ib, &[], wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, &counts).map(|(_, t)| t);
    let mut tlas_report = vec![];
    for (k, &n) in counts.iter().enumerate() {
        let per_row = ((n as f32 / blases.len() as f32).sqrt().ceil() as u32).max(1) | 1;
        let mut g = vec![];
        let mut w = vec![];
        let mut cpu = vec![];
        for _ in 0..REPEATS {
            let t0 = Instant::now();
            let tlas = make_tlas(device, &blases, n, step, per_row);
            cpu.push(t0.elapsed().as_secs_f64() * 1e3);
            let (gt, wt) = timed_build(&gpu, &[], Some(&tlas));
            g.extend(gt);
            w.push(wt);
        }
        let (gm, wm, cm) = (median(g), median(w), median(cpu));
        let mem = tlas_mem.as_ref().map(|t| t[k]);
        println!("TLAS build {n} instances: GPU {gm:.3} ms, wall {wm:.2} ms (instance fill on CPU {cm:.2} ms){}", mem.map(|m| format!(", {:.2} MB + {:.2} MB scratch", m.0 as f64 / 1e6, m.1 as f64 / 1e6)).unwrap_or_default());
        tlas_report.push(serde_json::json!({ "instances": n, "gpu_ms": gm, "wall_ms": wm, "cpu_fill_ms": cm, "bytes": mem.map(|m| m.0), "scratch_bytes": mem.map(|m| m.1) }));
    }
    // rebuild of a resident TLAS after moving every instance (what a refit
    // would do; wgpu 30 rebuilds in full, see the module docs).
    let refit = {
        let n = 1_000u32;
        let per_row = ((n as f32 / blases.len() as f32).sqrt().ceil() as u32).max(1) | 1;
        let mut tlas = make_tlas(device, &blases, n, step, per_row);
        let _ = timed_build(&gpu, &[], Some(&tlas));
        let mut g = vec![];
        let mut w = vec![];
        for r in 0..REPEATS {
            let t0 = Instant::now();
            for i in 0..n as usize {
                if let Some(inst) = tlas[i].as_mut() {
                    inst.transform[3] += 0.5 * (r as f32 + 1.0);
                }
            }
            let fill = t0.elapsed().as_secs_f64() * 1e3;
            let (gt, wt) = timed_build(&gpu, &[], Some(&tlas));
            g.extend(gt);
            w.push(wt + fill);
        }
        let (gm, wm) = (median(g), median(w));
        println!("TLAS rebuild after moving 1000 instances: GPU {gm:.3} ms, wall {wm:.2} ms");
        serde_json::json!({ "instances": n, "gpu_ms": gm, "wall_ms": wm })
    };

    // the traced scene: the real block plus copies round it, ~400 instances.
    let n_inst = blases.len() as u32 * 9;
    let tlas = make_tlas(device, &blases, n_inst, step, 3);
    let (_, w) = timed_build(&gpu, &[], Some(&tlas));
    let scene_tris = tris * 9;
    println!("traced TLAS: {n_inst} instances, {:.1} M instanced triangles (built in {w:.1} ms wall)", scene_tris as f64 / 1e6);

    // --- pipelines -----------------------------------------------------------
    let src = wgpu::ShaderModuleDescriptor { label: Some("rt-spike"), source: wgpu::ShaderSource::Wgsl(include_str!("rt_spike.wgsl").into()) };
    let t0 = Instant::now();
    let module = device.create_shader_module(src.clone());
    // SAFETY: the shader indexes only inside its buffers for in-range hits;
    // the unchecked variant measures the cost of naga's runtime checks
    // (bounds, ray-query initialisation tracking).
    let module_fast = unsafe { device.create_shader_module_trusted(src, wgpu::ShaderRuntimeChecks::unchecked()) };
    let vis = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
    let e = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
    let sbuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
    let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rt-bgl0"),
        entries: &[
            e(0, vis, wgpu::BindingType::AccelerationStructure { vertex_return: false }),
            e(1, vis, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            e(2, vis, sbuf),
            e(3, vis, sbuf),
            e(4, vis, sbuf),
        ],
    });
    let store = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba32Float, view_dimension: wgpu::TextureViewDimension::D2 };
    let sampled = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
    let bgl_cp = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-bgl-cp"), entries: &[e(0, wgpu::ShaderStages::COMPUTE, store)] });
    let bgl_cb = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-bgl-cb"), entries: &[e(1, wgpu::ShaderStages::COMPUTE, sampled), e(2, wgpu::ShaderStages::COMPUTE, store)] });
    let bgl_fb = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("rt-bgl-fb"), entries: &[e(1, wgpu::ShaderStages::FRAGMENT, sampled)] });
    let layout = |l: &[&wgpu::BindGroupLayout]| {
        let ls: Vec<Option<&wgpu::BindGroupLayout>> = l.iter().map(|x| Some(*x)).collect();
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &ls, immediate_size: 0 })
    };
    let (pl_cp, pl_cb, pl_fp, pl_fb) = (layout(&[&bgl0, &bgl_cp]), layout(&[&bgl0, &bgl_cb]), layout(&[&bgl0]), layout(&[&bgl0, &bgl_fb]));
    let compute = |m: &wgpu::ShaderModule, l: &wgpu::PipelineLayout, entry: &str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(l), module: m, entry_point: Some(entry), compilation_options: Default::default(), cache: None });
    let render = |l: &wgpu::PipelineLayout, entry: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(l),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba32Float, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None,
            cache: None,
        })
    };
    let cs_primary = compute(&module, &pl_cp, "cs_primary");
    let cs_bounce = compute(&module, &pl_cb, "cs_bounce");
    let cs_primary_fast = compute(&module_fast, &pl_cp, "cs_primary");
    let cs_bounce_fast = compute(&module_fast, &pl_cb, "cs_bounce");
    let fs_primary = render(&pl_fp, "fs_primary");
    let fs_bounce = render(&pl_fb, "fs_bounce");
    gpu.wait();
    let pipeline_s = t0.elapsed().as_secs_f64();
    let pipe_errors = gpu.errors();
    println!("pipelines (compute and fragment ray queries) created in {pipeline_s:.2} s; errors: {pipe_errors:?}");
    assert!(pipe_errors.is_empty(), "pipeline creation failed: {pipe_errors:?}");

    // --- targets -------------------------------------------------------------
    let tex = |label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let (prim, bnc, fprim, fbnc) = (tex("rt-prim"), tex("rt-bounce"), tex("rt-fprim"), tex("rt-fbounce"));
    let view = |t: &wgpu::Texture| t.create_view(&Default::default());
    let (prim_v, bnc_v, fprim_v, fbnc_v) = (view(&prim), view(&bnc), view(&fprim), view(&fbnc));
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-cam"), size: std::mem::size_of::<CamU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rt-bg0"),
        layout: &bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: tlas.as_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: vb.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: ib.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: gb.as_entire_binding() },
        ],
    });
    let bg_cp = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl_cp, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&prim_v) }] });
    let bg_cb = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl_cb, entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&prim_v) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&bnc_v) }] });
    let bg_fb = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl_fb, entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&prim_v) }] });
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rt-read"), size: (W * H * 16) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let read_tex = |t: &wgpu::Texture| -> Vec<[f32; 4]> {
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 16), rows_per_image: Some(H) } },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([enc.finish()]);
        bytemuck::pod_collect_to_vec(&gpu.read(&readback))
    };

    // --- traces ----------------------------------------------------------------
    let elev = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::ElevationGrid::load(&p).ok());
    let boxes_min_y = scene.boxes.iter().map(|b| b.0.y).fold(f32::MAX, f32::min);
    let ground = |x: f32, z: f32| elev.as_ref().map(|e| e.ground(x as f64, z as f64) as f32).unwrap_or(boxes_min_y);
    let street = street_camera(&scene, &ground);
    let c = (scene.min + scene.max) * 0.5;
    let overview = Camera::new(Vec3::new(c.x - 200.0, ground(c.x, c.z) + 220.0, c.z + 350.0), 0.5, -0.45, W as f32 / H as f32);

    let dispatch = |pipe: &wgpu::ComputePipeline, bg1: &wgpu::BindGroup| {
        gpu.timed(|enc, q| {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("rt-trace"), timestamp_writes: q.map(|q| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }) });
            p.set_pipeline(pipe);
            p.set_bind_group(0, &bg0, &[]);
            p.set_bind_group(1, bg1, &[]);
            p.dispatch_workgroups(W.div_ceil(8), H.div_ceil(8), 1);
        })
    };
    let draw = |pipe: &wgpu::RenderPipeline, target: &wgpu::TextureView, bg1: Option<&wgpu::BindGroup>| {
        gpu.timed(|enc, q| {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rt-trace-fs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: q.map(|q| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, &bg0, &[]);
            if let Some(b) = bg1 {
                p.set_bind_group(1, b, &[]);
            }
            p.draw(0..3, 0..1);
        })
    };
    let time = |f: &dyn Fn() -> (Option<f64>, f64)| -> f64 {
        let _ = f(); // warm-up
        median((0..REPEATS).map(|_| { let (g, w) = f(); g.unwrap_or(w) }).collect())
    };

    let mut traces = vec![];
    let out = out_dir();
    std::fs::create_dir_all(&out).expect("output folder");
    for (name, cam) in [("street", &street), ("overview", &overview)] {
        const RPP: u32 = 4;
        let set = |rays: u32, tmax: f32, flags: u32| gpu.queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&cam_u(cam, 1, rays, tmax, flags)));
        set(RPP, 100.0, 0);
        let cp = time(&|| dispatch(&cs_primary, &bg_cp));
        let cp_fast = time(&|| dispatch(&cs_primary_fast, &bg_cp));
        let _ = dispatch(&cs_primary, &bg_cp);
        let texels = read_tex(&prim);
        let hit_px = texels.iter().filter(|t| t[0] >= 0.0).count() as u64;
        let mean_t = texels.iter().filter(|t| t[0] >= 0.0).map(|t| t[0] as f64).sum::<f64>() / hit_px.max(1) as f64;
        // hemisphere rays from each primary hit: closest hit within 100 m
        // (bounce lighting), and any hit (occlusion / shadow style).
        let cb = time(&|| dispatch(&cs_bounce, &bg_cb));
        let cb_fast = time(&|| dispatch(&cs_bounce_fast, &bg_cb));
        let _ = dispatch(&cs_bounce, &bg_cb);
        let bounce_texels = read_tex(&bnc);
        set(RPP, 100.0, 4); // RAY_FLAG_TERMINATE_ON_FIRST_HIT (naga RayFlag bit 2)
        let cb_any = time(&|| dispatch(&cs_bounce_fast, &bg_cb));
        set(RPP, 1.0e4, 0);
        let cb_long = time(&|| dispatch(&cs_bounce_fast, &bg_cb));
        set(RPP, 100.0, 0);
        // the same from a fragment shader.
        let fp = time(&|| draw(&fs_primary, &fprim_v, None));
        let ftexels = read_tex(&fprim);
        let differ = texels.iter().zip(&ftexels).filter(|(a, b)| (a[0] - b[0]).abs() > 1e-3 * a[0].abs().max(1.0)).count();
        let fb = time(&|| draw(&fs_bounce, &fbnc_v, Some(&bg_fb)));
        let prim_rays = (W * H) as f64;
        let bounce_rays = (hit_px * RPP as u64) as f64;
        let rate = |rays: f64, ms: f64| rays / (ms * 1e-3) / 1e6;
        println!(
            "trace {name}: {:.1}% of pixels hit, mean primary distance {mean_t:.1} m\n  compute primary {cp:.2} ms ({:.0} Mrays/s; unchecked shader {cp_fast:.2} ms, {:.0} Mrays/s)\n  compute bounce x{RPP} closest<=100 m {cb:.2} ms ({:.0} Mrays/s; unchecked {cb_fast:.2} ms, {:.0} Mrays/s); any-hit {cb_any:.2} ms ({:.0} Mrays/s); closest<=10 km {cb_long:.2} ms ({:.0} Mrays/s)\n  fragment primary {fp:.2} ms ({:.0} Mrays/s; {differ} pixels differ from compute); fragment bounce {fb:.2} ms ({:.0} Mrays/s)",
            hit_px as f64 * 100.0 / prim_rays,
            rate(prim_rays, cp), rate(prim_rays, cp_fast), rate(bounce_rays, cb), rate(bounce_rays, cb_fast), rate(bounce_rays, cb_any), rate(bounce_rays, cb_long), rate(prim_rays, fp), rate(bounce_rays, fb)
        );
        traces.push(serde_json::json!({
            "view": name, "eye": cam.eye.to_array(), "yaw": cam.yaw, "pitch": cam.pitch, "hit_fraction": hit_px as f64 / prim_rays, "mean_primary_t": mean_t,
            "primary_rays": prim_rays, "bounce_rays": bounce_rays, "rays_per_pixel": RPP,
            "compute_primary_ms": cp, "compute_primary_unchecked_ms": cp_fast, "compute_bounce_ms": cb, "compute_bounce_unchecked_ms": cb_fast, "compute_bounce_anyhit_ms": cb_any, "compute_bounce_10km_ms": cb_long,
            "fragment_primary_ms": fp, "fragment_bounce_ms": fb, "fragment_vs_compute_differing_pixels": differ,
            "mrays_s": { "compute_primary": rate(prim_rays, cp), "compute_primary_unchecked": rate(prim_rays, cp_fast), "compute_bounce": rate(bounce_rays, cb), "compute_bounce_unchecked": rate(bounce_rays, cb_fast), "compute_bounce_anyhit": rate(bounce_rays, cb_any), "compute_bounce_10km": rate(bounce_rays, cb_long), "fragment_primary": rate(prim_rays, fp), "fragment_bounce": rate(bounce_rays, fb) }
        }));
        let sky = [0.55, 0.7, 0.9];
        let ln_far = (1.0f32 + 1500.0).ln();
        write_png(&out.join(format!("rt-{name}-distance.png")), W, H, &to_rgba(&texels, |t| if t[0] < 0.0 { sky } else { let v = 1.0 - (1.0 + t[0]).ln() / ln_far; [v, v, v] }));
        write_png(&out.join(format!("rt-{name}-normal.png")), W, H, &to_rgba(&texels, |t| if t[0] < 0.0 { sky } else { [t[1] * 0.5 + 0.5, t[2] * 0.5 + 0.5, t[3] * 0.5 + 0.5] }));
        write_png(&out.join(format!("rt-{name}-bounce-occlusion.png")), W, H, &to_rgba(&bounce_texels, |t| if t[0] < 0.0 { sky } else { let v = 1.0 - t[0]; [v, v, v] }));
        assert!(differ < (W * H / 1000) as usize, "fragment and compute primary hits disagree on {differ} pixels");
    }
    let trace_errors = gpu.errors();
    println!("errors during tracing: {trace_errors:?}");

    let report = serde_json::json!({
        "uptime": uptime,
        "adapter": { "name": info.name, "backend": format!("{:?}", info.backend), "driver": info.driver, "driver_info": info.driver_info },
        "features": { "ray_query": true, "vertex_return": af.contains(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN), "ray_tracing_pipelines": af.contains(wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES), "timestamps": gpu.timestamps.is_some() },
        "timing_source": if gpu.timestamps.is_some() { "GPU timestamps (builds: end of an empty compute pass before to start of one after)" } else { "wall clock" },
        "scene": { "blas_count": scene.geoms.len(), "triangles": tris, "vertices": scene.vertices.len(), "kit_dir": scene.kit_dir.file_name().map(|s| s.to_string_lossy().into_owned()), "kit_load_s": scene.load_s, "structures_s": scene.structures_s, "structures_triangles": scene.geoms.iter().find(|g| g.name == "structures").map(|g| g.index_count / 3), "traced_instances": n_inst, "traced_instanced_triangles": scene_tris },
        "blas_build": blas_report, "blas_single_tile": single, "tlas_build": tlas_report, "tlas_rebuild_moved": refit,
        "pipeline_create_s": pipeline_s, "traces": traces, "errors": trace_errors,
    });
    let p = out.join("rt-spike.json");
    std::fs::write(&p, serde_json::to_string_pretty(&report).unwrap()).expect("report");
    println!("wrote {}", p.display());
    assert!(trace_errors.is_empty(), "wgpu errors while tracing: {trace_errors:?}");
}
