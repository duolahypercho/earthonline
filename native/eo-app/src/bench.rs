//! Offscreen benchmark: fixed camera paths, no window, no vsync.

use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_render::camera::Camera;
use eo_render::{COUNTER_WORDS, GpuContext, GpuOptions, QUERIES_PER_FRAME, Renderer, Targets, hitch, read_mapped, wgpu};
use glam::Vec3;
use serde_json::json;
use std::time::Instant;

/// The game's spawn near Folsom and 4th, surveyed city metres (x east, z
/// south) at 1:1; `spawn()` is where it is on the scaled ground plan.
pub const SPAWN_SURVEYED: [f32; 2] = [1539.0, -366.0];
/// Eye height above ground for the street walk.
pub(crate) const EYE_HEIGHT_M: f32 = 1.7;
/// Walking speed, metres per frame at a nominal 60 frames per second.
const WALK_M_PER_FRAME: f32 = 1.4 / 60.0;
/// Length of the authored-street walk over one segment.
const AUTHORED_WALK_M: f32 = 150.0;
/// The kit street: Castro Street from 19th toward 18th (x east, z south,
/// surveyed at 1:1; `kit_street()` on the scaled plan), 478 m from the
/// nearest Blender-built building; 150 m of it (x the ground scale) are walked.
const KIT_STREET_SURVEYED: [[f32; 2]; 2] = [[-1348.0, 2246.0], [-1369.0, 1949.0]];
const KIT_WALK_M: f32 = 150.0;
/// The drive: Geary Boulevard westbound from x = -1600, at 60 km/h, the
/// frames paced to 60 Hz of wall time so the streamers get real time.
const DRIVE_STREET: &str = "Geary Boulevard";
const DRIVE_FROM_X: f32 = -1600.0;
const DRIVE_SPEED_MS: f32 = 60.0 / 3.6;
const DRIVE_EYE_M: f32 = 1.3;
const FRAME_PERIOD_S: f64 = 1.0 / 60.0;

/// the ground-plan scale as f32 (1.25 by default, the approved deviation).
pub fn gsf() -> f32 {
    eo_data::groundscale::scale() as f32
}

/// a surveyed (1:1) point on the scaled ground plan: x and z
/// scaled about the map origin, y (a height) kept.
pub fn on_plan(v: Vec3) -> Vec3 {
    Vec3::new(v.x * gsf(), v.y, v.z * gsf())
}

/// the spawn on the scaled ground plan (the same spot on the street).
pub fn spawn() -> [f32; 2] {
    eo_data::groundscale::plan32(SPAWN_SURVEYED)
}

/// the kit street on the scaled ground plan.
pub fn kit_street() -> [[f32; 2]; 2] {
    KIT_STREET_SURVEYED.map(eo_data::groundscale::plan32)
}

/// Direction of the Folsom Street segment nearest the spawn, oriented east.
fn folsom_heading(world: &World) -> Vec3 {
    let s = glam::Vec2::from(spawn()); // spawn()
    let mut best = (f32::MAX, glam::Vec2::new(1.0, -1.0));
    for r in world.city.roads.iter().filter(|r| r.name == "Folsom Street") {
        for w in r.points.windows(2) {
            let (a, b) = (glam::Vec2::from(w[0]), glam::Vec2::from(w[1]));
            let ab = b - a;
            let t = ((s - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (a + ab * t).distance(s);
            if d < best.0 {
                best = (d, ab);
            }
        }
    }
    let mut dir = best.1.normalize_or(glam::Vec2::new(1.0, -1.0).normalize());
    if dir.x < 0.0 {
        dir = -dir;
    }
    Vec3::new(dir.x, 0.0, dir.y)
}

/// close views beside the kit street (Castro) and the authored street
/// (Folsom): from the segment's camera at a fixed frame, step `side` metres
/// across the street and look square at the frontage (`turn` +-1 picks the
/// side), eye 1.7 m above the ground there, `pitch` up.
fn facade_cameras(segs: &[Segment], world: &World, far: f32) -> Vec<(&'static str, Camera)> {
    let mut out = vec![];
    let views: [(&str, &str, f32, f32, f32, f32); 6] = [
        // (name, segment, t along the walk, turn, metres across, pitch)
        ("kit-wall", "kit_street", 0.30, 1.0, 6.5, 0.10),
        ("kit-window", "kit_street", 0.30, 1.0, 6.5, 0.42),
        ("kit-shop", "kit_street", 0.62, -1.0, 6.5, 0.02),
        ("kit-wall-b", "kit_street", 0.80, -1.0, 6.5, 0.12),
        ("authored-window", "authored_street", 0.30, 1.0, 5.0, 0.30),
        ("authored-wall", "authored_street", 0.55, -1.0, 5.0, 0.08),
    ];
    for (name, seg, t, turn, across, pitch) in views {
        let Some(s) = segs.iter().find(|s| s.name == seg) else { continue };
        let n = 1024;
        let i = (t * n as f32) as usize;
        let a = (s.camera)(i, n).eye;
        let b = (s.camera)(i + 8, n).eye;
        let along = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or(Vec3::X);
        let side = Vec3::new(-along.z, 0.0, along.x) * turn;
        let mut p = a + side * across;
        // when the fixed step across lands in or against the building
        // (the street-size change moved the frontage), stand FACADE_STANDOFF_M in front of it.
        let at = |q: Vec3| Vec3::new(q.x, world.elevation.ground(q.x as f64, q.z as f64) as f32 + EYE_HEIGHT_M, q.z);
        let probe = Camera::new(at(p), yaw_of(side), 0.0, 1.0);
        if blocked_view(&probe, CAPTURE_WALL_WARN_M).is_some() {
            if let Some(d) = wall_ahead(at(a), side, 60.0) {
                p = a + side * (d - FACADE_STANDOFF_M);
                println!("capture: facade-{name}: {across} m across is in or against the building; now {FACADE_STANDOFF_M} m in front of it ({:.1} m across)", d - FACADE_STANDOFF_M);
            }
        }
        let eye = at(p);
        out.push((name, Camera { far, ..Camera::new(eye, yaw_of(side), pitch, (s.camera)(i, n).aspect) }));
    }
    out
}

fn yaw_of(dir: Vec3) -> f32 {
    // yaw 0 looks north (-z), positive toward east (+x).
    dir.x.atan2(-dir.z)
}

mod cc0_shots; // cc0
mod fix3_shots;
mod gs_shots;
mod hlod_shots; // hlod
mod far_shots;
mod landmarks_bench; // landmarks
mod occlusion_check; // occlusion
mod sky_shots; // sky
mod so_shots; // the sky and ocean review cards
mod street_bench;
mod view_bench;
mod water_bench; // water
pub(crate) use water_bench::dive_camera;
mod sign_shots; // signs

struct Segment {
    name: &'static str,
    description: String,
    camera: Box<dyn Fn(usize, usize) -> Camera>,
    /// Frames measured (after the warm-up).
    frames: usize,
    /// Pace frames to 60 Hz of wall time (the drive).
    paced: bool,
    /// Kit packs keep streaming during the measured frames (no settle wait inside).
    streams: bool,
}

/// Blender buildings whose footprint-frame centre (from the manifest, so the
/// path is the same with models on or off; moved with its record)
/// is within `lateral` metres of the walk from `start` along `dir` for `len` metres.
fn authored_along(world: &World, start: Vec3, dir: Vec3, len: f32, lateral: f32) -> usize {
    let list = eo_assets::load_blender_buildings(&world.paths.repo_root).unwrap_or_default();
    list.iter()
        .filter(|b| {
            let d = eo_data::groundscale::record_shift(b.record_key()).unwrap_or([0.0, 0.0]);
            let p = Vec3::new((b.frame.center.x + d[0]) as f32, 0.0, (-b.frame.center.z + d[1]) as f32) - start;
            let t = p.dot(dir);
            t >= 0.0 && t <= len && (p - dir * t).length() <= lateral
        })
        .count()
}

/// Kit buildings with their footprint centroid within `lateral` of a line.
fn kit_along(cents: &[Vec3], start: Vec3, dir: Vec3, len: f32, lateral: f32) -> usize {
    cents
        .iter()
        .filter(|c| {
            let p = **c - start;
            let t = p.dot(dir);
            t >= 0.0 && t <= len && (p - dir * t).length() <= lateral
        })
        .count()
}

/// Footprint centroids of the kit buildings.
fn kit_centroids(world: &World) -> Vec<Vec3> {
    let Some(k) = &world.kit else { return vec![] };
    world
        .city
        .buildings
        .iter()
        .enumerate()
        .filter(|(i, _)| k.building_group[*i] != eo_render::scene::NO_GROUP)
        .map(|(_, b)| {
            let (sx, sz) = b.ring.iter().fold((0.0f32, 0.0f32), |a, p| (a.0 + p[0], a.1 + p[1]));
            let n = b.ring.len() as f32;
            Vec3::new(sx / n, 0.0, sz / n)
        })
        .collect()
}

/// The drive path: Geary's points west of DRIVE_FROM_X, smoothed along x
/// (the boulevard is two carriageways), as an arc-length polyline.
fn drive_path(world: &World) -> Vec<Vec3> {
    let mut pts: Vec<[f32; 2]> = world
        .city
        .roads
        .iter()
        .filter(|r| r.name == DRIVE_STREET)
        .flat_map(|r| r.points.iter().copied())
        .filter(|p| p[0] <= DRIVE_FROM_X * gsf() + 50.0) // * gsf()
        .collect();
    pts.sort_by(|a, b| b[0].partial_cmp(&a[0]).unwrap());
    let mut out = vec![];
    let mut x = DRIVE_FROM_X * gsf(); // * gsf()
    let x_end = pts.last().map(|p| p[0]).unwrap_or(DRIVE_FROM_X * gsf() - 1000.0); // * gsf()
    while x > x_end {
        let near: Vec<f32> = pts.iter().filter(|p| (p[0] - x).abs() <= 40.0).map(|p| p[1]).collect();
        if !near.is_empty() {
            let z = near.iter().sum::<f32>() / near.len() as f32;
            out.push(Vec3::new(x, 0.0, z));
        }
        x -= 10.0;
    }
    out
}

fn along(path: &[Vec3], s: f32) -> (Vec3, Vec3) {
    let mut left = s;
    for w in path.windows(2) {
        let d = w[1] - w[0];
        let l = d.length();
        if left <= l {
            return (w[0] + d * (left / l.max(1e-6)), d.normalize_or(Vec3::NEG_X));
        }
        left -= l;
    }
    let n = path.len();
    (path[n - 1], (path[n - 1] - path[n - 2]).normalize_or(Vec3::NEG_X))
}

fn segments(world: &World, args: &Args, aspect: f32, far: f32) -> Vec<Segment> {
    let frames = args.frames;
    let heading = folsom_heading(world);
    let start = Vec3::new(spawn()[0], 0.0, spawn()[1]); // spawn()
    let ground: Vec<f32> = (0..=1024)
        .map(|i| {
            let p = start + heading * WALK_M_PER_FRAME * i as f32;
            world.elevation.ground(p.x as f64, p.z as f64) as f32
        })
        .collect();
    let base_yaw = yaw_of(heading);
    // the same orbit over the scaled plan (centre and radius scaled, altitude real).
    let center = on_plan(Vec3::new(1200.0, 0.0, -1200.0));
    let (radius, altitude) = (1800.0f32 * gsf(), 350.0f32);
    let (east, west) = (
        authored_along(world, start, heading, AUTHORED_WALK_M, 45.0),
        authored_along(world, start, -heading, AUTHORED_WALK_M, 45.0),
    );
    let walk_dir = if west > east { -heading } else { heading };
    let lined = east.max(west);
    let walk_yaw = yaw_of(walk_dir);
    let elev = &world.elevation;
    let walk_ground: Vec<f32> = (0..=1024)
        .map(|i| {
            let p = start + walk_dir * AUTHORED_WALK_M * (i as f32 / 1024.0);
            elev.ground(p.x as f64, p.z as f64) as f32
        })
        .collect();
    // Kit street.
    let ksp = kit_street(); // kit_street(), KIT_WALK_M * gsf() (the same stretch of the street)
    let kit_walk_m = KIT_WALK_M * gsf();
    let ks = Vec3::new(ksp[0][0], 0.0, ksp[0][1]);
    let kdir = (Vec3::new(ksp[1][0], 0.0, ksp[1][1]) - ks).normalize();
    let kit_yaw = yaw_of(kdir);
    let cents = kit_centroids(world);
    let kit_lined = kit_along(&cents, ks, kdir, kit_walk_m, 30.0);
    let kit_ground: Vec<f32> = (0..=1024)
        .map(|i| {
            let p = ks + kdir * kit_walk_m * (i as f32 / 1024.0);
            elev.ground(p.x as f64, p.z as f64) as f32
        })
        .collect();
    // Drive.
    let path = drive_path(world);
    let drive_frames = args.drive_frames;
    let drive_len = DRIVE_SPEED_MS * (drive_frames + args.warmup) as f32 * FRAME_PERIOD_S as f32;
    let lift = world.city.meta.street_design.as_ref().and_then(|s| s.road_surface_lift).unwrap_or(0.0) as f32;
    let drive_samples: Vec<(Vec3, Vec3)> = (0..drive_frames + args.warmup + 1)
        .map(|i| {
            let (p, d) = along(&path, DRIVE_SPEED_MS * i as f32 * FRAME_PERIOD_S as f32);
            let y = elev.ground(p.x as f64, p.z as f64) as f32 + lift + DRIVE_EYE_M;
            (Vec3::new(p.x, y, p.z), d)
        })
        .collect();
    let drive_start = drive_samples.first().map(|s| s.0).unwrap_or(Vec3::ZERO);
    let drive_end = drive_samples.last().map(|s| s.0).unwrap_or(Vec3::ZERO);
    let overview = eo_data::groundscale::plan32(OVERVIEW_CENTER);
    let drive_kit = path.windows(2).map(|w| kit_along(&cents, w[0], (w[1] - w[0]).normalize_or(Vec3::X), (w[1] - w[0]).length(), 40.0)).sum::<usize>();
    vec![
        Segment {
            name: "street_walk",
            description: format!(
                "eye {EYE_HEIGHT_M} m above ground at the spawn ({}, {}), walking 1.4 m/s (per frame at a nominal 60 fps) along the nearest Folsom Street segment while the view turns one full 360 degrees over the segment",
                spawn()[0], spawn()[1]
            ),
            camera: Box::new(move |i, n| {
                let p = start + heading * WALK_M_PER_FRAME * i as f32;
                let eye = Vec3::new(p.x, ground[i.min(1024)] + EYE_HEIGHT_M, p.z);
                let yaw = base_yaw + std::f32::consts::TAU * i as f32 / n as f32;
                Camera { far, ..Camera::new(eye, yaw, 0.0, aspect) }
            }),
            frames,
            paced: false,
            streams: false,
        },
        Segment {
            name: "elevated_sweep",
            description: format!(
                "orbit of radius {radius} m at {altitude} m around ({}, {}), looking at the orbit centre, one revolution over the segment",
                center.x, center.z
            ),
            camera: Box::new(move |i, n| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                let eye = center + Vec3::new(a.sin() * radius, altitude, a.cos() * radius);
                let to = (center + Vec3::new(0.0, 20.0, 0.0)) - eye;
                let yaw = yaw_of(to);
                let pitch = (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan();
                Camera { far, ..Camera::new(eye, yaw, pitch, aspect) }
            }),
            frames,
            paced: false,
            streams: true,
        },
        Segment {
            name: "city_overview",
            description: format!(
                "stress: eye {OVERVIEW_ALTITUDE_M} m above the city centre ({}, {}) looking almost straight down (pitch -1.45 rad), far plane past the ground, the view turning one revolution; the whole city is in view",
                overview[0], overview[1]
            ),
            camera: Box::new(move |i, n| {
                let eye = Vec3::new(overview[0], OVERVIEW_ALTITUDE_M, overview[1]);
                let yaw = std::f32::consts::TAU * i as f32 / n as f32;
                Camera { far: OVERVIEW_ALTITUDE_M * 1.5, ..Camera::new(eye, yaw, -1.45, aspect) }
            }),
            frames,
            paced: false,
            streams: false,
        },
        Segment {
            name: "authored_street",
            description: format!(
                "eye {EYE_HEIGHT_M} m above ground from the spawn ({}, {}) along Folsom Street ({}), {AUTHORED_WALK_M} m over the segment (faster than walking, to pass more frontage in the measured frames), the view swinging +-70 degrees across the street one and a half times; {lined} Blender-built buildings have their footprint-frame centre within 45 m of this line",
                spawn()[0],
                spawn()[1],
                if west > east { "west" } else { "east" }
            ),
            camera: Box::new(move |i, n| {
                let t = i as f32 / n.max(1) as f32;
                let p = start + walk_dir * AUTHORED_WALK_M * t;
                let eye = Vec3::new(p.x, walk_ground[((t * 1024.0) as usize).min(1024)] + EYE_HEIGHT_M, p.z);
                let yaw = walk_yaw + 70f32.to_radians() * (std::f32::consts::TAU * 1.5 * t).sin();
                Camera { far, ..Camera::new(eye, yaw, 0.04, aspect) }
            }),
            frames,
            paced: false,
            streams: false,
        },
        Segment {
            name: "kit_street",
            description: format!(
                "eye {EYE_HEIGHT_M} m above ground on Castro Street from ({}, {}) toward ({}, {}), {KIT_WALK_M} m over the segment, the view swinging +-70 degrees across the street one and a half times; {kit_lined} kit buildings have their footprint centroid within 30 m of this line; the nearest Blender-built building is 478 m away",
                ksp[0][0], ksp[0][1], ksp[1][0], ksp[1][1]
            ),
            camera: Box::new(move |i, n| {
                let t = i as f32 / n.max(1) as f32;
                let p = ks + kdir * kit_walk_m * t; // kit_walk_m
                let eye = Vec3::new(p.x, kit_ground[((t * 1024.0) as usize).min(1024)] + EYE_HEIGHT_M, p.z);
                let yaw = kit_yaw + 70f32.to_radians() * (std::f32::consts::TAU * 1.5 * t).sin();
                Camera { far, ..Camera::new(eye, yaw, 0.04, aspect) }
            }),
            frames,
            paced: false,
            streams: true,
        },
        Segment {
            name: "drive",
            description: format!(
                "{DRIVE_STREET} westbound from ({:.0}, {:.0}) to ({:.0}, {:.0}), {drive_len:.0} m at 60 km/h ({DRIVE_SPEED_MS:.2} m/s), eye {DRIVE_EYE_M} m above the carriageway, looking along the road; frames paced to 60 Hz of wall time; kit packs stream in and out during the measured frames (nothing is pre-loaded beyond the start); {drive_kit} kit buildings within 40 m of the route",
                drive_start.x, drive_start.z, drive_end.x, drive_end.z
            ),
            camera: Box::new(move |i, _n| {
                let (eye, d) = drive_samples[i.min(drive_samples.len() - 1)];
                Camera { far, ..Camera::new(eye, yaw_of(d), -0.02, aspect) }
            }),
            frames: drive_frames,
            paced: true,
            streams: true,
        },
    ]
    .into_iter()
    .chain(street_bench::sidewalk_segment(world, frames, aspect, far))
    .chain(water_bench::waterfront_segment(world, frames, aspect, far)) // water
    .chain(water_bench::bay_segment(world, frames, aspect, far)) // water
    .chain(water_bench::dive_segment(world, frames, aspect, far))
    .chain(so_shots::surf_segment(world, frames, aspect, far)) // opt-in (EO_SURF_BENCH=1), Ocean Beach's surf in view
    .chain(landmarks_bench::segments(world, frames, aspect, far)) // landmarks
    .chain(view_bench::turn_segments(world, frames, args.warmup, aspect, far))
    .collect()
}

/// Middle of the drawn buildings' extent (x -8348..5434, z -5964..8100), surveyed (`plan32` scales it).
const OVERVIEW_CENTER: [f32; 2] = [-1457.0, 1068.0];
const OVERVIEW_ALTITUDE_M: f32 = 18000.0;

/// with `Pace::Realtime` at most this many frames per camera position
/// (back-to-back frames of ~1.5-3 ms against 16.7 ms positions).
const REALTIME_FRAME_CAP: usize = 16;

/// Frames per timestamp query set (a set holds at most 4096 queries).
const TS_FRAMES: usize = (4096 / QUERIES_PER_FRAME) as usize;

/// The whole-frame target at 1920x1080 on the reference desktop GPU.
pub const FRAME_BUDGET_MS: f64 = 10.0;
/// Segments the budget check enforces (median frame total).
const STANDARD_SEGMENTS: &[&str] = &["street_walk", "elevated_sweep", "authored_street", "kit_street", "sidewalk_walk"];

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = (p / 100.0 * (s.len() - 1) as f64).round() as usize;
    s[rank.min(s.len() - 1)]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({ "median": percentile(v, 50.0), "p95": percentile(v, 95.0), "p99": percentile(v, 99.0), "min": percentile(v, 0.0), "max": percentile(v, 100.0), "n": v.len() })
}

const CAVEAT: &str = "Slice renderer only: the game's Blender-built building exteriors (hash-verified GLBs, kit materials baked as the JS bakes them), the kit (archetype) buildings planned by a Rust port of the JS planner (near rung 0, mid rung 2, footprint boxes far), extruded footprints for anything not yet streamed, (slice 4) the street surface ported from the JS street-surface-v2 (carriageway, gutters, kerbs, kerb ramps, footways, frontage, banks, junction pads, corner returns, lane paint, crosswalks, limit lines) near and flat strips far, buildings on their street-front floor with a foundation skirt, 8 m terrain with a coarse far level, a sea plane; the JS game's light rig, analytic sky, distance fog, 3 sun shadow cascades, HDR + ACES + grade (eo-look). No AO, bloom, clouds, interiors, landmarks, pedestrian-only paths and plazas, street furniture, vegetation or physics; (agents) the simulated cars and people (eo-sim, the JS game's traffic bodies and the Rocketbox people) are drawn unless --no-agents. This is NOT the game's frame time.";

/// Stream around `cam` until everything it wants is resident (or `timeout_s`).
fn settle(ctx: &GpuContext, renderer: &Renderer, world: &World, models: &mut ModelRuntime, cam: &Camera, timeout_s: f64) -> Result<f64> {
    settle_p(ctx, renderer, world, models, None, cam, timeout_s)
}

/// `settle`, also until the street props around `cam` are resident.
fn settle_p(ctx: &GpuContext, renderer: &Renderer, world: &World, models: &mut ModelRuntime, mut props: Option<&mut crate::props::PropsRuntime>, cam: &Camera, timeout_s: f64) -> Result<f64> {
    let t = Instant::now();
    models.set_camera(cam.eye);
    let mut last = Instant::now(); // the view clock moves with the wall while settling
    loop {
        let now = Instant::now();
        renderer.observe_view(cam, Some(view_bench::advance((now - last).as_secs_f64())));
        last = now;
        models.pump(ctx, renderer, world, cam.eye)?;
        if let Some(p) = props.as_deref_mut() {
            p.pump(ctx, renderer, &mut models.uploads, cam.eye, None, 0.0); // props
        }
        // Keep the GPU fed so the LOD state follows (and queued writes land).
        renderer.update(&ctx.queue, cam);
        eo_render::pace::submit_tick(&ctx.device, &ctx.queue); // always counted (an empty submit counts only with queued writes)
        // Reclaim finished submissions' staging memory, as a presenting loop would.
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        if models.settled() && props.as_deref().is_none_or(|p| p.settled()) { // props
            break;
        }
        if t.elapsed().as_secs_f64() > timeout_s {
            // say what never settled instead of passing silently.
            eprintln!("settle: timed out after {timeout_s:.0} s at eye ({:.0}, {:.0}, {:.0}): {}", cam.eye.x, cam.eye.y, cam.eye.z, models.unsettled());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

pub fn run(args: &Args, process_start: Instant) -> Result<()> {
    // the render thread above normal (workers lower themselves when started).
    let raised = args.perf.thread_priority && hitch::raise_render_thread();
    eo_render::alloc::claim_render_thread(); // this thread allocates from a heap no worker takes
    // the check compares two frames pixel for pixel; the AO pass
    // (surfaces) differs between two identical frames on its own (about 0.7 M
    // pixels of 2 M with occlusion off on both), so the check runs without it
    // and compares the AO pass's inputs (depth, HDR scene) instead.
    let check_args;
    let args = if args.perf.occlusion_check {
        // and without the sky slice's temporal stages (jitter, rain, the moving clouds).
        check_args = Args { look: eo_render::look::LookOptions { ao: false, sky: eo_render::sky::SkyOptions::off(), ..args.look }, ..args.clone() };
        &check_args
    } else {
        args
    };
    eprintln!("hitch: render thread priority {} (raised: {raised}); thread cycles per ms {:.0}", hitch::thread_priority(), hitch::cycles_per_ms());
    let mut world = load_world(args)?;
    if let Some(s) = &world.street {
        let _ = CAPTURE_BUILDINGS.set(s.world.clone()); // say when a camera stands inside a building
    }
    let load_data_ms = process_start.elapsed().as_secs_f64() * 1e3;
    let opts = GpuOptions {
        indirect_validation: args.indirect_validation,
        force_no_indirect_count: args.no_indirect_count,
        ..Default::default()
    };
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts)?;
    let (w, h) = (args.width, args.height);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let t_r = Instant::now();
    // the check renders object ids (ground truth).
    let (renderer, bases) = make_renderer(&ctx, &world, args, format, args.perf.occlusion_check)?;
    eprintln!("startup: gpu ready at {:.0} ms, renderer {:.0} ms", t_r.duration_since(process_start).as_secs_f64() * 1e3, t_r.elapsed().as_secs_f64() * 1e3);
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let targets = Targets::new(&ctx.device, w, h, format, args.msaa);
    let (attach, resolve) = match &targets.msaa {
        Some(m) => (m, Some(&color_view)),
        None => (&color_view, None),
    };
    let aspect = w as f32 / h as f32;
    let t_s = Instant::now();
    let mut segs = segments(&world, args, aspect, args.far);
    // the night, rain and fog paths (the same cameras under another hour or weather).
    if !args.perf.occlusion_check {
        segs.extend(sky_shots::extra_segments(segments(&world, args, aspect, args.far)));
    }
    eprintln!("startup: segments {:.0} ms", t_s.elapsed().as_secs_f64() * 1e3);
    if let Some(only) = &args.segments {
        segs.retain(|s| only.iter().any(|o| o == s.name));
    }
    let first_cam = (segs[0].camera)(0, args.warmup + segs[0].frames);
    let mut models = ModelRuntime::start(&mut world, args, bases, first_cam.eye, process_start);
    // the simulation, feed and agent library, live while the paths run.
    let mut agents = crate::agents::AgentRuntime::start(&world, args, crate::agents::AgentRuntime::focus_of(first_cam.eye, first_cam.forward()));
    let mut props = crate::props::PropsRuntime::start(&world, args); // props
    let mut audio = crate::audio::AudioRuntime::start(&world, eo_audio::Output::Null); // the null output (a bench never sounds); `--no-audio` for A/B

    // First frame: the load-time end point (footprints stand in for everything streamed).
    {
        renderer.update(&ctx.queue, &first_cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        // a debug-ids renderer (the check) draws into an ids target too.
        let ids = args.perf.occlusion_check.then(|| {
            ctx.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("first-frame-ids"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: args.msaa,
                    dimension: wgpu::TextureDimension::D2,
                    format: eo_render::IDS_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        renderer.encode_with_ids(&mut enc, attach, resolve, &targets.depth, ids.as_ref(), None);
        eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    }
    let first_frame_ms = process_start.elapsed().as_secs_f64() * 1e3;
    // Stream the models, the maps and the first path's kit packs in before measuring.
    let first_settle_ms = settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &first_cam, 600.0)?;
    if !agents.wait_ready(&ctx, &renderer, crate::agents::AgentRuntime::focus_of(first_cam.eye, first_cam.forward()), 300.0) {
        eprintln!("agents: not ready after 300 s ({:?}); paths run without them", agents.errors); // agents
    }
    // matched captures (`--capture-props2` chooses the poses from the data and writes
    // them; `EO_CAPTURE_POSES=<file>` renders a pose file): each pose settled, then captured.
    let poses_env = std::env::var("EO_CAPTURE_POSES").ok();
    if poses_env.is_some() || args.props2.capture {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        std::fs::create_dir_all(&dir)?;
        let poses = match &poses_env {
            Some(f) => crate::props::read_poses(std::path::Path::new(f))?,
            None => {
                let mut list = vec![];
                if std::env::var_os("EO_CAPTURE_SIGNS").is_some() {
                    list.extend(sign_shots::poses(&world, aspect, args.far)); // the sign capture card instead
                } else {
                    for (seg_name, frame) in [("kit_street", 260usize), ("sidewalk_walk", 195)] {
                        if let Some(seg) = segs.iter().find(|s| s.name == seg_name) {
                            list.push((format!("{seg_name}-frame{frame}"), (seg.camera)(frame, args.warmup + seg.frames), format!("bench path {seg_name}, frame {frame}")));
                        }
                    }
                    // `EO_GS_CAPTURE=1` writes only the two bench-path frames (the
                    // matched ground-scale pictures); otherwise the props2 poses follow as before.
                    if std::env::var("EO_GS_CAPTURE").is_err() {
                        list.extend(crate::props::props2_poses(&world, aspect, args.far));
                    }
                }
                let json: Vec<serde_json::Value> = list.iter().map(|(n, c, w)| crate::props::pose_json(n, c, w)).collect();
                let p = dir.join("capture-props2-poses.json");
                std::fs::write(&p, serde_json::to_string_pretty(&json)?)?;
                println!("wrote {}", p.display());
                for (n, c, w) in &list {
                    println!("  pose {n}: eye ({:.1}, {:.2}, {:.1}) yaw {:.3} pitch {:.3}: {w}", c.eye.x, c.eye.y, c.eye.z, c.yaw, c.pitch);
                }
                list.into_iter().map(|(name, camera, _)| crate::props::CapturePose { name, camera, player: None }).collect()
            }
        };
        let label = args.label.as_deref().unwrap_or("props2");
        for crate::props::CapturePose { name, camera: cam, player } in poses {
            let refresh_player = player.is_some() || !agents.extras.is_empty();
            agents.extras.clear();
            if let Some(p) = player {
                let lib = agents.lib.as_ref().ok_or_else(|| anyhow!("player scale reference requires the people library"))?;
                let body = eo_agents::feed::player_body(lib, p.feet, p.yaw, 0., 0., 0.)
                    .ok_or_else(|| anyhow!("no verified player avatar available for scale reference"))?;
                agents.extras.push(body);
                println!("scale reference {name}: {} at {:?}, model height {:?} m, people scale {}", lib.meta.model_names[lib.meta.player_model as usize], p.feet, lib.meta.family_heights.first(), body.scale);
            }
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam);
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            if refresh_player {
                agents.pump(&ctx, &renderer, crate::agents::AgentRuntime::focus_of(cam.eye, cam.forward()), 0.0);
            }
            let p = dir.join(format!("capture-{label}-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {}", p.display());
        }
        return Ok(());
    }
    let streamed_ms = process_start.elapsed().as_secs_f64() * 1e3;
    let ms = models.stats.clone();
    let kit_at_start = models.kit.as_ref().map(|k| (k.stats.clone(), k.arena_use()));
    println!(
        "models: {} resident ({} from cache, {} cooked from GLB, {} failed), {:.2} M triangles; kit maps baked in {:.0} ms; models and the first path's kit packs resident {:.0} ms after process start (first frame at {:.0} ms, settle {:.0} ms)",
        ms.resident,
        ms.from_cache,
        ms.cooked,
        ms.failed.len(),
        ms.triangles_resident as f64 / 1e6,
        ms.kit_bake_ms.unwrap_or(f64::NAN),
        streamed_ms,
        first_frame_ms,
        first_settle_ms
    );
    if let Some((ks, _)) = &kit_at_start {
        println!(
            "kit at the first path: {} near + {} mid packs resident, {:.2} M triangles, {:.1} MB vertices + {:.1} MB indices; {} packs from cache, {} cooked ({} buildings planned)",
            ks.resident_near,
            ks.resident_mid,
            ks.resident_triangles as f64 / 1e6,
            ks.resident_vertex_bytes as f64 / 1e6,
            ks.resident_index_bytes as f64 / 1e6,
            ks.packs_from_cache,
            ks.packs_cooked,
            ks.cooked_buildings
        );
    }
    for (id, e) in &ms.failed {
        eprintln!("  model failed: {id}: {e}");
    }
    // `--cc0-shots` writes the CC0 slice's matched capture set instead of the timing.
    if cc0_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return cc0_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("cc0"));
    }
    // `--sky-shots` writes the sky slice's capture set instead of the timing.
    if sky_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return sky_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("shots"));
    }
    // `--so-shots` writes the sky and ocean review cards instead of the timing.
    if so_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return so_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("so"));
    }
    // `--gs-shots` writes the street size's matched capture set instead of the timing.
    if gs_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return gs_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("gs"));
    }
    // `--far-shots` writes the far city's fixed card set instead of the timing.
    if far_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return far_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("far"));
    }
    // `--hlod-shots` writes the far layer's capture set instead of the timing.
    if hlod_shots::requested() {
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        return hlod_shots::run(&ctx, &renderer, &world, &mut models, &mut props, &mut agents, &segs, args, &color, attach, resolve, &targets, &dir, args.label.as_deref().unwrap_or("shots"));
    }
    // `--occlusion-check` runs the ground-truth test instead of the timing.
    if args.perf.occlusion_check {
        if args.msaa != 1 {
            anyhow::bail!("--occlusion-check runs without MSAA");
        }
        let (report, ok) = occlusion_check::run(&ctx, &renderer, &world, &mut models, &segs, args.warmup, w, h)?;
        let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
        std::fs::create_dir_all(&dir)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let path = dir.join(format!("occlusion-check-{}-{stamp}.json", args.label.as_deref().unwrap_or("default")));
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
        println!("wrote {}", path.display());
        if !ok {
            anyhow::bail!("occlusion check failed (see above)");
        }
        return Ok(());
    }

    let period_ns = ctx.queue.get_timestamp_period() as f64;
    let mut seg_reports = vec![];
    let mut budget_failures: Vec<String> = vec![];
    let mut max_failures: Vec<String> = vec![]; // hitch
    let budget_ms = args.budget_ms.unwrap_or(FRAME_BUDGET_MS);
    let cw = COUNTER_WORDS as u64 * 4;
    let mut kit_after_paths = vec![];
    for seg in &segs {
        // camera positions along the path; with `Pace::Realtime` the
        // drive's camera follows the wall clock and frames are rendered back
        // to back, so more frames than positions are drawn (up to `total`).
        let path_frames = args.warmup + seg.frames;
        let (paced, realtime) = match args.perf.pace {
            crate::Pace::Realtime => (false, seg.paced),
            crate::Pace::Hz60 => (seg.paced, false),
            crate::Pace::All60 => (true, false),
            crate::Pace::None => (false, false),
        };
        let total = if realtime { path_frames * REALTIME_FRAME_CAP } else { path_frames };
        // a night, rain or fog path runs under its own hour and weather.
        let sky_restore = sky_shots::enter(&renderer, args, seg.name);
        // Settle at the path's start (the drive streams from there on).
        let mut vd = view_bench::PathTrack::begin(&models, &props);
        let settle_ms = settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &(seg.camera)(0, path_frames), 600.0)?;
        // the path's view clock is its camera time from here.
        vd.settled(&models, &props);
        let path_t0 = view_bench::clock();
        let streams = seg.streams || args.perf.stream_all_paths;
        let mut vd_caps = view_bench::capture_frames(seg.name);
        let chunks = total.div_ceil(TS_FRAMES);
        let qsets: Vec<wgpu::QuerySet> = if ctx.timestamps {
            (0..chunks)
                .map(|_| ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("frame-timestamps"), ty: wgpu::QueryType::Timestamp, count: QUERIES_PER_FRAME * TS_FRAMES as u32 }))
                .collect()
        } else {
            vec![]
        };
        // the props' own passes (cascade cull, casters, wires), a query set beside each.
        let pq = eo_render::props::QUERIES_PER_FRAME;
        let props_qsets: Vec<wgpu::QuerySet> = if ctx.timestamps && renderer.props.get().is_some() {
            (0..chunks).map(|_| ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("props-timestamps"), ty: wgpu::QueryType::Timestamp, count: pq * TS_FRAMES as u32 })).collect()
        } else {
            vec![]
        };
        let stats_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stats-readback"),
            size: total as u64 * cw,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // shadow casters drawn per cascade (commands, triangles).
        let scw = eo_render::look::SHADOW_COUNTER_WORDS as u64 * 4;
        let shadow_stats_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow-stats-readback"),
            size: total as u64 * scw,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // the patch cull's counters per frame (drawn, in the frustum, land, occluded).
        let water_stats_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-stats-readback"),
            size: total as u64 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let water_before = renderer.water.get().map(|w| w.stats());
        let mut cpu_ms = Vec::with_capacity(total);
        let mut encode_ms = Vec::with_capacity(total);
        let mut submit_ms = Vec::with_capacity(total);
        let mut pump_ms = Vec::with_capacity(total);
        let mut pump_bytes = Vec::with_capacity(total);
        let mut starts = Vec::with_capacity(total + 1);
        let mut subs: Vec<wgpu::SubmissionIndex> = Vec::with_capacity(total);
        // per-frame breakdown, the thread's on-CPU time, the wait for
        // the frame two back, and (EO_HITCH_ALLOC=1) the allocator's blocks.
        let mut traces: Vec<FrameRecord> = Vec::with_capacity(total);
        let alloc_trace = std::env::var("EO_HITCH_ALLOC").is_ok();
        let mut warmup = args.warmup;
        let mut cam_last = 0;
        let mut idle_ms = 0.0; // the last frame's wait for the GPU (prepay included)
        let seg_start = Instant::now();
        for i in 0..total {
            let cam_i = if realtime { (seg_start.elapsed().as_secs_f64() / FRAME_PERIOD_S) as usize } else { i };
            if cam_i >= path_frames {
                break;
            }
            if realtime && cam_i < args.warmup {
                warmup = i + 1;
            }
            cam_last = cam_i;
            if paced {
                let due = seg_start + std::time::Duration::from_secs_f64(i as f64 * FRAME_PERIOD_S);
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                }
            }
            // when the driver's submission spike is due, it is taken
            // here by tiny submissions, if the wait the last frame had can
            // absorb it (`eo_render::pace`). A prepay the GPU did not outlast
            // (the wait after it under PREPAY_FREE_WAIT_MS) is charged to this
            // frame's CPU time.
            let tpre = Instant::now();
            let prepay = if args.perf.submit_pacing && i >= 2 { eo_render::pace::prepay(&ctx.device, &ctx.queue, idle_ms) } else { Default::default() };
            let prepay_ms = tpre.elapsed().as_secs_f64() * 1e3;
            // Two frames in flight, as a double-buffered swapchain would allow.
            let tw = Instant::now();
            if i >= 2 {
                ctx.device
                    .poll(wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None })
                    .map_err(|e| anyhow!("{e}"))?;
            }
            let wait_ms = tw.elapsed().as_secs_f64() * 1e3;
            let overran = prepay.submissions > 0 && wait_ms < PREPAY_FREE_WAIT_MS;
            let charged_ms = if overran { prepay_ms } else { 0.0 };
            idle_ms = if overran { wait_ms } else { prepay_ms + wait_ms };
            let alloc = if alloc_trace { ctx.device.generate_allocator_report().map(|a| (a.blocks.len() as u32, a.total_reserved_bytes)) } else { None };
            hitch::take();
            let c0 = hitch::thread_cycles();
            let t0 = Instant::now();
            starts.push(t0);
            let cam = (seg.camera)(cam_i, path_frames);
            renderer.observe_view(&cam, Some(path_t0 + cam_i as f64 * FRAME_PERIOD_S)); // camera time
            // Streaming on the render thread: queue arrivals, write <= the upload budget.
            let pf = if streams {
                models.set_camera(cam.eye);
                models.pump(&ctx, &renderer, &world, cam.eye)?
            } else {
                Default::default()
            };
            // with the realtime drive the simulation advances by the wall time of the last frame.
            let sim_dt = if realtime { starts.len().checked_sub(2).map(|k| (t0 - starts[k]).as_secs_f64()).unwrap_or(FRAME_PERIOD_S).min(0.1) } else { FRAME_PERIOD_S };
            if props.on {
                // tiles, lamps on the simulation clock, the parked cars to the agent renderer.
                hitch::timed(hitch::Phase::PropsPump, || props.pump(&ctx, &renderer, &mut models.uploads, cam.eye, Some(agents.sim_time), sim_dt)); // timed
                if !streams {
                    let _s = hitch::span(hitch::Phase::Uploads); // timed
                    let _ = models.uploads.pump(&renderer, &ctx.queue);
                }
                agents.parked = hitch::timed(hitch::Phase::ParkedAgents, || props.parked_agents(agents.lib.as_deref(), cam.eye).to_vec()); // timed
            }
            hitch::timed(hitch::Phase::Agents, || agents.pump(&ctx, &renderer, crate::agents::AgentRuntime::focus_of(cam.eye, cam.forward()), sim_dt)); // agents (counted in the pump)
            audio.frame(crate::audio::FrameIn::camera(&cam, sim_dt, &mut agents, &renderer, &world.elevation)); // audio (counted in the pump)
            let tp = Instant::now();
            if let Some(w) = renderer.water.get() {
                w.set_time(cam_i as f64 * FRAME_PERIOD_S); // the sea's clock follows the path's
            }
            hitch::timed(hitch::Phase::CameraUpdate, || renderer.update(&ctx.queue, &cam));
            let enc_span = hitch::span(hitch::Phase::Encode);
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            let ts = qsets.get(i / TS_FRAMES).map(|q| (q, QUERIES_PER_FRAME * (i % TS_FRAMES) as u32));
            if let Some(p) = renderer.props.get() {
                p.set_timestamps(props_qsets.get(i / TS_FRAMES).map(|q| (q.clone(), pq * (i % TS_FRAMES) as u32)));
            }
            renderer.encode(&mut enc, attach, resolve, &targets.depth, ts);
            enc.copy_buffer_to_buffer(renderer.counter_buffer(), 0, &stats_buf, i as u64 * cw, cw);
            enc.copy_buffer_to_buffer(renderer.look.shadow_counter_buffer(), 0, &shadow_stats_buf, i as u64 * scw, scw);
            if let Some(w) = renderer.water.get().filter(|_| renderer.water_active()) {
                enc.copy_buffer_to_buffer(w.counter_buffer(), 0, &water_stats_buf, i as u64 * 16, 16); // water
            }
            drop(enc_span);
            let cmd = hitch::timed(hitch::Phase::Finish, || enc.finish());
            let t1 = Instant::now();
            subs.push(hitch::timed(hitch::Phase::Submit, || eo_render::pace::submit(&ctx.queue, [cmd]))); // counted
            let t2 = Instant::now();
            let c1 = hitch::thread_cycles();
            traces.push(FrameRecord { trace: hitch::take(), on_cpu_ms: hitch::on_cpu_ms(c0, c1), wait_ms, alloc, prepay: (prepay, prepay_ms, charged_ms) }); // + prepay
            pump_ms.push((tp - t0).as_secs_f64() * 1e3);
            pump_bytes.push(pf.bytes as f64);
            encode_ms.push((t1 - tp).as_secs_f64() * 1e3);
            submit_ms.push((t2 - t1).as_secs_f64() * 1e3);
            cpu_ms.push((t2 - t0).as_secs_f64() * 1e3 + charged_ms); // + a prepay the wait did not absorb
            // what is on screen at a lower level than its zone wants (outside the frame's timing).
            vd.frame(&world, &models, &cam, cam_i as f64 * FRAME_PERIOD_S * 1e3, pf.bytes, cam_i >= args.warmup);
            if vd_caps.first().is_some_and(|&f| cam_i >= f) {
                let f = vd_caps.remove(0);
                let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
                std::fs::create_dir_all(&dir)?;
                let p = dir.join(format!("capture-vd-{}-{}-f{f}.png", args.label.as_deref().unwrap_or("default"), seg.name));
                let c = (seg.camera)(f, path_frames);
                renderer.observe_view(&c, Some(path_t0 + f as f64 * FRAME_PERIOD_S));
                capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &c, &p)?;
                println!("wrote {} (path frame {f}, drawn at frame {cam_i}: the streaming state of that moment)", p.display());
            }
        }
        ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
        starts.push(Instant::now());
        view_bench::set_clock(path_t0 + cam_last as f64 * FRAME_PERIOD_S);
        let vd_json = vd.finish(&models, &props, args.perf.view_detail.enabled);
        sky_shots::leave(&renderer, sky_restore); // sky
        if let Some(p) = renderer.props.get() {
            p.set_timestamps(None);
        }
        // frames actually drawn (realtime: as many as the clock allowed).
        let total = cpu_ms.len();
        // the props' own passes, per measured frame.
        let (mut pr_cull, mut pr_casters, mut pr_wires) = (vec![], vec![], vec![]);
        if !props_qsets.is_empty() {
            let mut ts: Vec<u64> = vec![];
            for q in props_qsets.iter().take(total.div_ceil(TS_FRAMES)) {
                let n = pq * TS_FRAMES as u32;
                let resolve_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("props-ts-resolve"), size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
                let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("props-ts-read"), size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let mut enc = ctx.device.create_command_encoder(&Default::default());
                enc.resolve_query_set(q, 0..n, &resolve_buf, 0);
                enc.copy_buffer_to_buffer(&resolve_buf, 0, &read_buf, 0, n as u64 * 8);
                eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
                ts.extend(read_mapped(&ctx, &read_buf)?.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())));
            }
            let ms = |a: u64, b: u64| if b >= a { (b - a) as f64 * period_ns / 1e6 } else { 0.0 };
            for i in warmup.min(total)..total {
                let b = i * pq as usize;
                if b + 5 < ts.len() {
                    pr_cull.push(ms(ts[b], ts[b + 1]));
                    pr_casters.push(ms(ts[b + 2], ts[b + 3]));
                    pr_wires.push(ms(ts[b + 4], ts[b + 5]));
                }
            }
        }
        if realtime {
            println!(
                "\n[{}] realtime: {total} frames drawn ({warmup} warm-up) over {:.1} s of wall time for {path_frames} camera positions (last {cam_last}); frames rendered back to back",
                seg.name,
                seg_start.elapsed().as_secs_f64()
            );
        }
        // From here on `args.warmup` is this segment's warm-up frame count.
        let args = &Args { warmup, ..args.clone() };
        let interval_ms: Vec<f64> = (warmup..total).map(|i| (starts[i + 1] - starts[i]).as_secs_f64() * 1e3).collect();
        let stats_bytes = read_mapped(&ctx, &stats_buf)?;
        let stats: Vec<u32> = eo_render::scene::cast_u32(&stats_bytes);
        let word = |i: usize, k: usize| stats[i * COUNTER_WORDS + k] as f64;
        let range = args.warmup..total;
        let drawn_objects: Vec<f64> = range.clone().map(|i| word(i, 0) + word(i, 1) + word(i, 10)).collect(); // + late opaque
        // what the frustum + LOD pass would draw, and the early pass's share.
        let frustum_objects: Vec<f64> = range.clone().map(|i| word(i, 11)).collect();
        let frustum_tris: Vec<f64> = range.clone().map(|i| word(i, 12)).collect();
        let early_objects: Vec<f64> = range.clone().map(|i| word(i, 13)).collect();
        let late_new_objects: Vec<f64> = range.clone().map(|i| word(i, 10)).collect();
        let occluded_objects: Vec<f64> = range.clone().map(|i| word(i, 14)).collect();
        let drawn_blend: Vec<f64> = range.clone().map(|i| word(i, 1)).collect();
        let drawn_tris: Vec<f64> = range.clone().map(|i| word(i, 2)).collect();
        let model_parts: Vec<f64> = range.clone().map(|i| word(i, 3)).collect();
        let model_tris: Vec<f64> = range.clone().map(|i| word(i, 4)).collect();
        let standins: Vec<f64> = range.clone().map(|i| word(i, 5)).collect();
        let kit_objs: Vec<f64> = range.clone().map(|i| word(i, 6)).collect();
        let kit_tris: Vec<f64> = range.clone().map(|i| word(i, 7)).collect();
        let street_objs: Vec<f64> = range.clone().map(|i| word(i, 8)).collect();
        let street_tris: Vec<f64> = range.clone().map(|i| word(i, 9)).collect();
        let sstats: Vec<u32> = eo_render::scene::cast_u32(&read_mapped(&ctx, &shadow_stats_buf)?);
        let sw = eo_render::look::SHADOW_COUNTER_WORDS;
        let shadow_cmds: Vec<f64> = range.clone().map(|i| (0..3).map(|c| sstats[i * sw + c] as f64).sum()).collect();
        let shadow_tris: Vec<f64> = range.clone().map(|i| (3..6).map(|c| sstats[i * sw + c] as f64).sum()).collect();
        // patches drawn / in the frustum / culled as land / occluded per frame.
        let wstats: Vec<u32> = eo_render::scene::cast_u32(&read_mapped(&ctx, &water_stats_buf)?);
        let water_patch = |k: usize| -> Vec<f64> { range.clone().map(|i| wstats[i * 4 + k] as f64).collect() };
        let (water_drawn, water_frustum, water_land, water_occluded) = (water_patch(0), water_patch(1), water_patch(2), water_patch(3));
        let water_frames = renderer.water.get().map(|w| w.stats()).zip(water_before).map(|(a, b)| (a.frames - b.frames, a.fft_frames - b.fft_frames, a.copy_frames - b.copy_frames, a.fogged_frames - b.fogged_frames));

        let mut gpu = json!(null);
        let mut gpu_total_ms = vec![];
        let (mut cull, mut draw) = (vec![], vec![]);
        let (mut sh_cull, mut sh_draw, mut post) = (vec![], vec![], vec![]);
        let mut ao_pass = vec![]; // surfaces
        let mut probe_pass = vec![]; // facade
        let mut sky_passes: [Vec<f64>; 6] = Default::default(); // the sky passes, then the cloud-shadow map
        let (mut water_fft, mut water_copy, mut water_pass) = (vec![], vec![], vec![]); // water
        let (mut occ_ms, mut late_draw, mut early_draw) = (vec![], vec![], vec![]); // occlusion
        if !qsets.is_empty() {
            let mut ts: Vec<u64> = vec![];
            for q in qsets.iter().take(total.div_ceil(TS_FRAMES)) {
                let n = QUERIES_PER_FRAME * TS_FRAMES as u32;
                let resolve_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ts-resolve"),
                    size: n as u64 * 8,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let read_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ts-read"),
                    size: n as u64 * 8,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut enc = ctx.device.create_command_encoder(&Default::default());
                enc.resolve_query_set(q, 0..n, &resolve_buf, 0);
                enc.copy_buffer_to_buffer(&resolve_buf, 0, &read_buf, 0, n as u64 * 8);
                eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
                let bytes = read_mapped(&ctx, &read_buf)?;
                ts.extend(bytes.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())));
            }
            let ms = |a: u64, b: u64| b.wrapping_sub(a) as f64 * period_ns / 1e6;
            let (qo, ql) = (eo_render::Q_OCCLUSION as usize, eo_render::Q_LATE_DRAW as usize); // occlusion
            for i in args.warmup..total {
                let b = i * QUERIES_PER_FRAME as usize;
                cull.push(ms(ts[b], ts[b + 1]));
                // the city is the early pass + the late pass.
                early_draw.push(ms(ts[b + 2], ts[b + 3]));
                late_draw.push(ms(ts[b + ql], ts[b + ql + 1]));
                occ_ms.push(ms(ts[b + qo], ts[b + qo + 1]));
                draw.push(ms(ts[b + 2], ts[b + 3]) + ms(ts[b + ql], ts[b + ql + 1]));
                sh_cull.push(ms(ts[b + 4], ts[b + 5]));
                sh_draw.push(ms(ts[b + 6], ts[b + 7]));
                post.push(ms(ts[b + 8], ts[b + 9]));
                ao_pass.push(ms(ts[b + 10], ts[b + 11])); // occlusion + bounce (before post)
                let qp = eo_render::Q_PROBE as usize; // the scene probe's face (+ fill on a capture's last face)
                probe_pass.push(ms(ts[b + qp], ts[b + qp + 1]));
                // lights, fx (rain, luminaires, velocity), volume, resolve, bloom.
                for (k, q) in [eo_render::sky::Q_SKY_LIGHTS, eo_render::sky::Q_SKY_FX, eo_render::sky::Q_SKY_VOLUME, eo_render::sky::Q_SKY_RESOLVE, eo_render::sky::Q_SKY_BLOOM, eo_render::sky::Q_CLOUD_SHADOW].iter().enumerate() {
                    sky_passes[k].push(ms(ts[b + *q as usize], ts[b + *q as usize + 1]));
                }
                // FFT (+ plain cull), the copy (the gap before the water pass), the water pass (+ the glass after the water).
                let (qf, qw) = (eo_render::Q_WATER_FFT as usize, eo_render::Q_WATER as usize);
                water_fft.push(ms(ts[b + qf], ts[b + qf + 1]));
                water_copy.push(ms(ts[b + qf + 1], ts[b + qw]));
                water_pass.push(ms(ts[b + qw], ts[b + qw + 1]));
                // Frame order: cull, shadow cull, shadow depth, city (early), pyramid + late cull, city (late) + sky, AO, post.
                gpu_total_ms.push(ms(ts[b], ts[b + 9]));
            }
            let probe_json = {
                let p = renderer.probe.stats(); // facade
                json!({ "captures": p.captures, "faces": p.faces, "transfer": p.transfer, "frames_per_capture": p.last_capture_frames, "look_transfer_in_use": renderer.look.transfer() })
            };
            gpu = json!({
                "frame_ms_cull_begin_to_post_end": summary(&gpu_total_ms),
                "cull_pass_ms (lod + cull dispatches)": summary(&cull),
                "draw_pass_ms (city + sky into HDR)": summary(&draw),
                "occlusion_early_draw_ms": summary(&early_draw),
                "occlusion_pyramid_and_late_cull_ms": summary(&occ_ms),
                "occlusion_late_draw_ms (late opaque + sky + glass)": summary(&late_draw),
                "shadow_cull_pass_ms": summary(&sh_cull),
                "shadow_depth_passes_ms (3 cascades)": summary(&sh_draw),
                "post_pass_ms (tone map, grade, finish)": summary(&post),
                "ao_pass_ms (surfaces: occlusion + bounce, half resolution)": summary(&ao_pass),
                "probe_pass_ms (facade: scene probe face cull + draw, fill/levels on a capture's last face; 0 when idle)": summary(&probe_pass),
                "probe": probe_json,
                // sky
                "sky_lights_pass_ms (gather + clusters)": summary(&sky_passes[0]),
                "sky_fx_pass_ms (luminaires, rain, agent velocity)": summary(&sky_passes[1]),
                "sky_volume_pass_ms (shafts, marine layer, clouds; half resolution)": summary(&sky_passes[2]),
                "sky_resolve_pass_ms (volume composite + TAA)": summary(&sky_passes[3]),
                "sky_bloom_pass_ms": summary(&sky_passes[4]),
                "sky_cloud_shadow_pass_ms (the cloud-shadow map)": summary(&sky_passes[5]),
                "sky": sky_shots::describe(&renderer),
                // water
                "water_fft_pass_ms (FFT when water was seen lately, + the patch cull without occlusion)": summary(&water_fft),
                "water_copy_ms (the frame copy: the gap before the water pass)": summary(&water_copy),
                "water_pass_ms (the sea, then the glass drawn after it)": summary(&water_pass),
                "shadow_casters_drawn_per_frame (3 cascades summed)": summary(&shadow_cmds),
                "shadow_triangles_drawn_per_frame (3 cascades summed)": summary(&shadow_tris),
            });
        }
        // Whole frame = CPU (streaming pump + encode + submit) + GPU (cull
        // begin -> draw end), summed per frame as if nothing overlapped.
        let cpu_meas = &cpu_ms[args.warmup..];
        let frame_total: Vec<f64> =
            if gpu_total_ms.is_empty() { interval_ms.clone() } else { cpu_meas.iter().zip(&gpu_total_ms).map(|(c, g)| c + g).collect() };
        let (tot_med, tot_p95, tot_max) = (percentile(&frame_total, 50.0), percentile(&frame_total, 95.0), percentile(&frame_total, 100.0));
        let verdict = |v: f64| if v < budget_ms { "PASS" } else { "FAIL" };
        let enc_m = &encode_ms[args.warmup..];
        let sub_m = &submit_ms[args.warmup..];
        let pump_m = &pump_ms[args.warmup..];
        let pump_b = &pump_bytes[args.warmup..];
        let worst = frame_total.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(k, _)| k).unwrap_or(0);
        let budget = json!({
            "budget_ms": budget_ms,
            "frame_total_definition": if gpu_total_ms.is_empty() { "wall interval (no timestamps)" } else { "CPU (streaming pump + encode + submit) + GPU cull-begin..draw-end, summed per frame" },
            "frame_total_ms": summary(&frame_total),
            "median_verdict": verdict(tot_med),
            "p95_verdict": verdict(tot_p95),
            "max_verdict": verdict(tot_max),
            "headroom_ms_at_median": budget_ms - tot_med,
            "headroom_ms_at_p95": budget_ms - tot_p95,
            "worst_frame": {
                "frame": worst + args.warmup,
                "total_ms": frame_total.get(worst),
                "cpu_pump_ms": pump_m.get(worst),
                "cpu_encode_ms": enc_m.get(worst),
                "cpu_submit_ms": sub_m.get(worst),
                "gpu_ms": gpu_total_ms.get(worst),
                "upload_bytes": pump_b.get(worst),
            },
            "used_split_ms_median": {
                "gpu_culling": percentile(&cull, 50.0),
                "gpu_draw": percentile(&draw, 50.0),
                "gpu_shadow_cull": percentile(&sh_cull, 50.0),
                "gpu_shadow_depth": percentile(&sh_draw, 50.0),
                "gpu_post": percentile(&post, 50.0),
                "cpu_streaming_pump": percentile(pump_m, 50.0),
                "cpu_encode": percentile(enc_m, 50.0),
                "cpu_submit": percentile(sub_m, 50.0),
            },
            "used_split_ms_p95": {
                "gpu_culling": percentile(&cull, 95.0),
                "gpu_draw": percentile(&draw, 95.0),
                "gpu_shadow_cull": percentile(&sh_cull, 95.0),
                "gpu_shadow_depth": percentile(&sh_draw, 95.0),
                "gpu_post": percentile(&post, 95.0),
                "cpu_streaming_pump": percentile(pump_m, 95.0),
                "cpu_encode": percentile(enc_m, 95.0),
                "cpu_submit": percentile(sub_m, 95.0),
            },
        });
        println!(
            "\n[{}] BUDGET {budget_ms} ms: frame total median {tot_med:.3} [{}]  p95 {tot_p95:.3} [{}]  MAX {tot_max:.3} [{}]\n    split (median ms): GPU cull {:.3}, GPU draw {:.3}, CPU pump {:.3}, CPU encode {:.3}, CPU submit {:.3}; uploads/frame median {:.0} B, max {:.0} B",
            seg.name,
            verdict(tot_med),
            verdict(tot_p95),
            verdict(tot_max),
            percentile(&cull, 50.0),
            percentile(&draw, 50.0),
            percentile(pump_m, 50.0),
            percentile(enc_m, 50.0),
            percentile(sub_m, 50.0),
            percentile(pump_b, 50.0),
            percentile(pump_b, 100.0),
        );
        audio.segment_report(seg.name, warmup); // audio
        println!(
            "    look (median / p95 ms): shadow cull {:.3} / {:.3}, shadow depth (3 cascades) {:.3} / {:.3}, ao {:.3} / {:.3}, probe {:.3} / {:.3} max {:.3}, post {:.3} / {:.3}; shadow casters {:.0}, shadow triangles {:.2} M",
            percentile(&sh_cull, 50.0),
            percentile(&sh_cull, 95.0),
            percentile(&sh_draw, 50.0),
            percentile(&sh_draw, 95.0),
            percentile(&ao_pass, 50.0),
            percentile(&ao_pass, 95.0),
            percentile(&probe_pass, 50.0),
            percentile(&probe_pass, 95.0),
            percentile(&probe_pass, 100.0),
            percentile(&post, 50.0),
            percentile(&post, 95.0),
            percentile(&shadow_cmds, 50.0),
            percentile(&shadow_tris, 50.0) / 1e6,
        );
        // sky
        if !sky_passes[0].is_empty() {
            println!(
                "    sky (median / p95 ms): lights {:.3} / {:.3}, fx {:.3} / {:.3}, volume {:.3} / {:.3}, resolve {:.3} / {:.3}, bloom {:.3} / {:.3}, cloud shadow {:.3} / {:.3}",
                percentile(&sky_passes[0], 50.0),
                percentile(&sky_passes[0], 95.0),
                percentile(&sky_passes[1], 50.0),
                percentile(&sky_passes[1], 95.0),
                percentile(&sky_passes[2], 50.0),
                percentile(&sky_passes[2], 95.0),
                percentile(&sky_passes[3], 50.0),
                percentile(&sky_passes[3], 95.0),
                percentile(&sky_passes[4], 50.0),
                percentile(&sky_passes[4], 95.0),
                percentile(&sky_passes[5], 50.0),
                percentile(&sky_passes[5], 95.0),
            );
        }
        // water
        println!(
            "    water {}: patches drawn median {:.0} / p95 {:.0} (in frustum {:.0}, land {:.0}, occluded {:.0}); FFT {:.3} / {:.3}, copy {:.3} / {:.3}, water pass (+ glass) {:.3} / {:.3} ms (median / p95); FFT ran {} and the copy {} of {} frames, {} wholly past the fog (not drawn)",
            if renderer.water_active() { "ON" } else { "OFF" },
            percentile(&water_drawn, 50.0),
            percentile(&water_drawn, 95.0),
            percentile(&water_frustum, 50.0),
            percentile(&water_land, 50.0),
            percentile(&water_occluded, 50.0),
            percentile(&water_fft, 50.0),
            percentile(&water_fft, 95.0),
            percentile(&water_copy, 50.0),
            percentile(&water_copy, 95.0),
            percentile(&water_pass, 50.0),
            percentile(&water_pass, 95.0),
            water_frames.map(|f| f.1).unwrap_or(0),
            water_frames.map(|f| f.2).unwrap_or(0),
            water_frames.map(|f| f.0).unwrap_or(0),
            water_frames.map(|f| f.3).unwrap_or(0),
        );
        // what the frustum alone would draw, and what was drawn.
        println!(
            "    occlusion {}: objects in frustum median {:.0} -> drawn {:.0} (early {:.0} + late new {:.0}; occluded {:.0}); triangles in frustum {:.2} M -> drawn {:.2} M; GPU early draw {:.3}, pyramid + late cull {:.3}, late draw {:.3} ms (medians)",
            if renderer.occlusion_on() { "ON" } else { "OFF" },
            percentile(&frustum_objects, 50.0),
            percentile(&drawn_objects, 50.0),
            percentile(&early_objects, 50.0),
            percentile(&late_new_objects, 50.0),
            percentile(&occluded_objects, 50.0),
            percentile(&frustum_tris, 50.0) / 1e6,
            percentile(&drawn_tris, 50.0) / 1e6,
            percentile(&early_draw, 50.0),
            percentile(&occ_ms, 50.0),
            percentile(&late_draw, 50.0),
        );
        // the props' own GPU passes.
        if !pr_cull.is_empty() {
            println!(
                "    props (median / p95 / max ms): cascade cull {:.3} / {:.3} / {:.3}, casters {:.3} / {:.3} / {:.3}, wires {:.3} / {:.3} / {:.3} (props camera cull: inside pyramid + late cull; props draw: inside late draw)",
                percentile(&pr_cull, 50.0),
                percentile(&pr_cull, 95.0),
                percentile(&pr_cull, 100.0),
                percentile(&pr_casters, 50.0),
                percentile(&pr_casters, 95.0),
                percentile(&pr_casters, 100.0),
                percentile(&pr_wires, 50.0),
                percentile(&pr_wires, 95.0),
                percentile(&pr_wires, 100.0),
            );
        }
        // every frame over the budget, and the worst frames, with where the time went.
        let meas = &traces[args.warmup..];
        let gpu_parts = |k: usize| -> Vec<(&'static str, f64)> {
            if cull.is_empty() {
                return vec![];
            }
            let mut v = vec![
                ("gpu_cull", cull[k]),
                ("gpu_early_draw", early_draw[k]),
                ("gpu_shadow_cull", sh_cull[k]),
                ("gpu_shadow_depth", sh_draw[k]),
                ("gpu_pyramid_late_cull", occ_ms[k]),
                ("gpu_late_draw", late_draw[k]),
                ("gpu_ao", ao_pass[k]),
                ("gpu_post", post[k]),
                ("gpu_water_fft", water_fft[k]), // water
                ("gpu_water_copy", water_copy[k]),
                ("gpu_water_and_glass", water_pass[k]),
                // every timed pass of the frame, and what no pass accounts for
                ("gpu_probe", probe_pass[k]),
                ("gpu_sky_lights", sky_passes[0][k]),
                ("gpu_sky_fx", sky_passes[1][k]),
                ("gpu_sky_volume", sky_passes[2][k]),
                ("gpu_sky_resolve", sky_passes[3][k]),
                ("gpu_sky_bloom", sky_passes[4][k]),
                ("gpu_cloud_shadow", sky_passes[5][k]),
                ("gpu_props_cascade_cull", pr_cull.get(k).copied().unwrap_or(0.0)),
                ("gpu_props_casters", pr_casters.get(k).copied().unwrap_or(0.0)),
                ("gpu_props_wires", pr_wires.get(k).copied().unwrap_or(0.0)),
            ];
            let timed: f64 = v.iter().map(|p| p.1).sum();
            v.push(("gpu_untimed (agents compute/casters, gaps, other processes)", (gpu_total_ms[k] - timed).max(0.0)));
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            v
        };
        let mut order: Vec<usize> = (0..frame_total.len()).collect();
        order.sort_by(|a, b| frame_total[*b].partial_cmp(&frame_total[*a]).unwrap());
        let keep = order.iter().take_while(|&&k| frame_total[k] >= budget_ms).count().max(args.perf.worst_frames).min(order.len());
        let mut worst_frames = vec![];
        let mut over = 0;
        for &k in &order[..keep] {
            let r = &meas[k];
            let cpu = cpu_meas[k];
            let gpu_ms = gpu_total_ms.get(k).copied().unwrap_or(f64::NAN);
            let gp = gpu_parts(k);
            let (top, top_ms) = r.trace.top();
            let ran = r.on_cpu_ms + r.prepay.2; // a charged prepay ran on the CPU
            let cause = if gpu_ms > cpu {
                format!("GPU {:.2} ms (longest pass {} {:.2})", gpu_ms, gp.first().map(|g| g.0).unwrap_or("?"), gp.first().map(|g| g.1).unwrap_or(f64::NAN))
            } else if ran < 0.6 * cpu && cpu - ran > 1.0 {
                format!("render thread off the CPU: wall {cpu:.2} ms, ran {ran:.2} ms (longest phase {top} {top_ms:.2})")
            } else {
                format!("render thread CPU {cpu:.2} ms (ran {ran:.2}; longest phase {top} {top_ms:.2})")
            };
            if frame_total[k] >= budget_ms {
                over += 1;
                // + the frame's marks and bytes by target
                println!(
                    "    HITCH frame {}: total {:.2} ms = CPU {:.2} + GPU {:.2}; {cause}\n      phases {:?}; writes {} ({} B, largest {} B, {:.2} ms); arrivals {}, evictions {}, jobs done {}, cancelled {}; wait for frame-2 {:.2} ms; gpu {:?}{}\n      marks {:?}; bytes by target {:?}",
                    k + args.warmup,
                    frame_total[k],
                    cpu,
                    gpu_ms,
                    r.trace.phases().iter().map(|(n, v)| format!("{n} {v:.2}")).collect::<Vec<_>>(),
                    r.trace.write_calls,
                    r.trace.write_bytes,
                    r.trace.largest_write,
                    r.trace.write_ms,
                    r.trace.arrivals,
                    r.trace.evictions,
                    r.trace.jobs_done,
                    r.trace.jobs_cancelled,
                    r.wait_ms,
                    gp.iter().filter(|(_, v)| *v >= 0.05).map(|(n, v)| format!("{n} {v:.2}")).collect::<Vec<_>>(), // every pass over 0.05 ms
                    r.alloc
                        .map(|(b, res)| {
                            // what the frame added to the allocator (its record is taken before the frame, the next one's after it).
                            let next = meas.get(k + 1).and_then(|n| n.alloc);
                            format!("; allocator blocks {b}, reserved {:.1} MB{}", res as f64 / 1e6, next.map(|(nb, nr)| format!(" (this frame: {:+} blocks, {:+.1} MB)", nb as i64 - b as i64, (nr as f64 - res as f64) / 1e6)).unwrap_or_default())
                        })
                        .unwrap_or_default(),
                    r.trace.marks(),
                    r.trace.target_bytes(),
                );
            }
            let mut phases = serde_json::Map::new();
            for (n, v) in r.trace.phases() {
                phases.insert(format!("{n}_ms"), json!(v));
            }
            worst_frames.push(json!({
                "frame": k + args.warmup,
                "total_ms": frame_total[k],
                "cpu_wall_ms": cpu,
                "cpu_on_thread_ms": ran,
                "gpu_ms": gpu_ms,
                "cause": cause,
                "cpu_phases": phases,
                "gpu_passes_ms": gp.iter().map(|(n, v)| json!({n.to_string(): v})).collect::<Vec<_>>(),
                "writes": { "calls": r.trace.write_calls, "bytes": r.trace.write_bytes, "largest": r.trace.largest_write, "ms": r.trace.write_ms },
                "streaming": { "arrivals": r.trace.arrivals, "evictions": r.trace.evictions, "jobs_done": r.trace.jobs_done, "jobs_cancelled": r.trace.jobs_cancelled },
                "wait_for_frame_minus_2_ms": r.wait_ms,
                "submission_prepay": json!({"tiny_submissions": r.prepay.0.submissions, "took_the_spike": r.prepay.0.spiked, "ms": r.prepay.1, "charged_ms": r.prepay.2}),
                "allocator": r.alloc.map(|(b, res)| json!({"blocks": b, "reserved_bytes": res, "after_frame": meas.get(k + 1).and_then(|n| n.alloc).map(|(nb, nr)| json!({"blocks": nb, "reserved_bytes": nr}))})), // + after
                // the frame's named events and its bytes by target
                "marks": r.trace.marks().into_iter().map(|(n, v)| (n.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
                "write_bytes_by_target": r.trace.target_bytes().into_iter().map(|(n, v)| (n.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
            }));
        }
        // EO_HITCH_CSV=DIR writes every measured frame's breakdown (DIR/frames-<label>-<path>.csv).
        if let Ok(dir) = std::env::var("EO_HITCH_CSV") {
            let mut csv = String::from("frame,total_ms,cpu_ms,on_cpu_ms,gpu_ms,wait_ms,write_bytes,write_ms,prepay_ms,prepay_submissions,prepay_spiked,prepay_charged_ms");
            for n in hitch::PHASE_NAMES {
                csv.push_str(&format!(",{n}"));
            }
            for n in hitch::MARK_NAMES {
                csv.push_str(&format!(",{n}"));
            }
            csv.push('\n');
            for (k, r) in meas.iter().enumerate() {
                csv.push_str(&format!("{},{:.4},{:.4},{:.4},{:.4},{:.4},{},{:.4}", k + args.warmup, frame_total[k], cpu_meas[k], r.on_cpu_ms, gpu_total_ms.get(k).copied().unwrap_or(f64::NAN), r.wait_ms, r.trace.write_bytes, r.trace.write_ms));
                csv.push_str(&format!(",{:.4},{},{},{:.4}", r.prepay.1, r.prepay.0.submissions, r.prepay.0.spiked as u8, r.prepay.2));
                for v in r.trace.ms {
                    csv.push_str(&format!(",{v:.4}"));
                }
                for v in r.trace.marks {
                    csv.push_str(&format!(",{v}"));
                }
                csv.push('\n');
            }
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("frames-{}-{}.csv", args.label.as_deref().unwrap_or("default"), seg.name)), csv);
        }
        // with EO_HITCH_ALLOC=1, every measured frame that grew the GPU allocator, and its frame total.
        if alloc_trace {
            let grew: Vec<String> = (0..meas.len().saturating_sub(1))
                .filter_map(|k| {
                    let ((b0, r0), (b1, r1)) = (meas[k].alloc?, meas[k + 1].alloc?);
                    (b1 != b0 || r1 != r0).then(|| format!("f{} {:+}blk {:+.1}MB ({:.2} ms)", k + args.warmup, b1 as i64 - b0 as i64, (r1 as f64 - r0 as f64) / 1e6, frame_total[k]))
                })
                .collect();
            println!("    allocator changed in {} measured frames: {:?}", grew.len(), grew.iter().take(40).collect::<Vec<_>>());
        }
        let off_cpu: Vec<f64> = meas.iter().zip(cpu_meas).map(|(r, c)| (c - r.prepay.2 - r.on_cpu_ms).max(0.0)).collect(); // a charged prepay is not off-CPU time
        println!(
            "    hitch: {over} frames >= {budget_ms} ms; render thread off-CPU time per frame median {:.3} / max {:.3} ms; priority {}",
            percentile(&off_cpu, 50.0),
            percentile(&off_cpu, 100.0),
            hitch::thread_priority()
        );
        // the driver's submission spike: where it fell in the measured frames.
        let in_frames: u64 = meas.iter().map(|r| r.trace.marks[hitch::Mark::SubmitSpike as usize]).sum();
        let pre: Vec<&FrameRecord> = meas.iter().filter(|r| r.prepay.0.submissions > 0).collect();
        let taken = pre.iter().filter(|r| r.prepay.0.spiked).count();
        let charged = pre.iter().filter(|r| r.prepay.2 > 0.0).count();
        let max_prepay = pre.iter().map(|r| r.prepay.1).fold(0.0, f64::max);
        let min_wait_after = pre.iter().filter(|r| r.prepay.0.spiked).map(|r| r.wait_ms).fold(f64::INFINITY, f64::min);
        println!(
            "    submission spike: {in_frames} inside frames, {taken} taken in the wait for the GPU (prepay frames {}, tiny submissions {}, longest prepay {max_prepay:.2} ms, shortest wait after a taken spike {:.2} ms, charged to their frame {charged}){}",
            pre.len(),
            pre.iter().map(|r| r.prepay.0.submissions as u64).sum::<u64>(),
            if min_wait_after.is_finite() { min_wait_after } else { 0.0 },
            if args.perf.submit_pacing { "" } else { "; pacing off" },
        );
        let pacing_json = json!({
            "on": args.perf.submit_pacing,
            "spikes_inside_frames": in_frames,
            "spikes_taken_in_wait": taken,
            "prepay_frames": pre.len(),
            "tiny_submissions": pre.iter().map(|r| r.prepay.0.submissions as u64).sum::<u64>(),
            "longest_prepay_ms": max_prepay,
            "shortest_wait_after_taken_spike_ms": if min_wait_after.is_finite() { json!(min_wait_after) } else { json!(null) },
            "prepays_charged_to_their_frame": charged,
        });
        if STANDARD_SEGMENTS.contains(&seg.name) && tot_med >= budget_ms {
            budget_failures.push(format!("{} median {tot_med:.3} ms", seg.name));
        }
        // the MAX check (every path), enforced with --check-max.
        if tot_max >= budget_ms {
            max_failures.push(format!("{} MAX {tot_max:.3} ms", seg.name));
        }
        let kit_now = models.kit.as_ref().map(|k| {
            let s = &k.stats;
            let a = k.arena_use();
            json!({
                "resident_near_packs": s.resident_near, "resident_mid_packs": s.resident_mid,
                "resident_triangles": s.resident_triangles, "resident_vertex_bytes": s.resident_vertex_bytes, "resident_index_bytes": s.resident_index_bytes,
                "arena_vertices_used_peak": [a.0, a.1], "arena_indices_used_peak": [a.2, a.3], "arena_objects_used_peak": [a.4, a.5],
                "packs_from_cache": s.packs_from_cache, "packs_cooked": s.packs_cooked, "packs_uploaded": s.packs_uploaded, "packs_evicted": s.packs_evicted,
                "packs_dropped_unneeded": s.packs_dropped_unneeded, "packs_skipped_arena_full": s.packs_skipped_arena_full,
                "cooked_buildings": s.cooked_buildings, "unplanned": s.unplanned, "worker_ms_sum": s.worker_ms,
                "requested_now": k.requested(),
            })
        });
        kit_after_paths.push(json!({ "after": seg.name, "kit": kit_now }));
        let mut report = json!({
            "name": seg.name,
            "budget": budget,
            "camera_path": seg.description,
            "frames_measured": total - args.warmup,
            "pace": format!("{:?}", args.perf.pace), // hitch
            "realtime_camera": realtime,
            "warmup_frames": args.warmup,
            "paced_60hz": paced,
            "streams_during_measurement": streams, // + --stream-all-paths
            "settle_before_ms": settle_ms,
            "cpu_encode_submit_ms": summary(&cpu_ms[args.warmup..]),
            "cpu_streaming_pump_ms": summary(pump_m),
            "upload_bytes_per_frame": summary(pump_b),
            "upload_budget_bytes": args.upload_budget,
            "wall_interval_ms_two_frames_in_flight": summary(&interval_ms),
            "gpu": gpu,
            "objects_drawn_per_frame": summary(&drawn_objects),
            "transparent_objects_drawn_per_frame": summary(&drawn_blend),
            "triangles_drawn_per_frame": summary(&drawn_tris),
            "model_parts_drawn_per_frame": summary(&model_parts),
            "model_triangles_drawn_per_frame": summary(&model_tris),
            "footprint_standins_drawn_per_frame": summary(&standins),
            "kit_objects_drawn_per_frame": summary(&kit_objs),
            "kit_triangles_drawn_per_frame": summary(&kit_tris),
            "kit_after": kit_now,
            "street_objects_drawn_per_frame": summary(&street_objs),
            "street_triangles_drawn_per_frame": summary(&street_tris),
            "street_after": models.street.as_ref().map(street_bench::stats_json),
            // water
            "water": {
                "on": renderer.water_active(),
                "water_optics": args.look.water_optics,
                "patches_drawn_per_frame": summary(&water_drawn),
                "patches_in_frustum_per_frame": summary(&water_frustum),
                "patches_culled_land_per_frame": summary(&water_land),
                "patches_culled_occluded_per_frame": summary(&water_occluded),
                "frames": water_frames.map(|f| f.0), "fft_frames": water_frames.map(|f| f.1), "copy_frames": water_frames.map(|f| f.2), "fogged_frames": water_frames.map(|f| f.3),
            },
            // occlusion
            "occlusion": {
                "on": renderer.occlusion_on(),
                "objects_in_frustum_per_frame": summary(&frustum_objects),
                "triangles_in_frustum_per_frame": summary(&frustum_tris),
                "objects_drawn_per_frame": summary(&drawn_objects),
                "triangles_drawn_per_frame": summary(&drawn_tris),
                "objects_drawn_early_per_frame": summary(&early_objects),
                "objects_drawn_late_new_per_frame": summary(&late_new_objects),
                "objects_occluded_per_frame": summary(&occluded_objects),
                "gpu_early_draw_ms": summary(&early_draw),
                "gpu_pyramid_and_late_cull_ms": summary(&occ_ms),
                "gpu_late_draw_ms": summary(&late_draw),
            },
            "props_passes_ms": {
                "cascade_cull": summary(&pr_cull),
                "casters": summary(&pr_casters),
                "wires": summary(&pr_wires),
            },
            // hitch
            "hitch": {
                "frames_over_budget": over,
                "paced_60hz_actual": paced,
                "render_thread_priority": hitch::thread_priority(),
                "render_thread_off_cpu_ms_per_frame": summary(&off_cpu),
                "wait_for_frame_minus_2_ms": summary(&meas.iter().map(|r| r.wait_ms).collect::<Vec<_>>()),
                "submission_spike": pacing_json,
                "worst_frames": worst_frames,
            },
        });
        report["view_detail"] = vd_json; // outside the json! above: its recursion limit
        println!("  street objects/frame   median {:.0}  p95 {:.0}  (street triangles median {:.0}, p95 {:.0})", percentile(&street_objs, 50.0), percentile(&street_objs, 95.0), percentile(&street_tris, 50.0), percentile(&street_tris, 95.0));
        println!(
            "\n[{}] {} frames after {} warm-up (settle {:.0} ms)\n  CPU pump+encode+submit ms median {:.3}  p95 {:.3}  max {:.3}\n  wall ms/frame          median {:.3}  p95 {:.3}\n  GPU ms (cull->draw)    median {}  p95 {}\n  objects drawn/frame    median {:.0}  p95 {:.0}  (transparent median {:.0})\n  triangles drawn/frame  median {:.0}  p95 {:.0}\n  model parts/frame      median {:.0}  (model triangles median {:.0}, footprint stand-ins median {:.0})\n  kit objects/frame      median {:.0}  p95 {:.0}  (kit triangles median {:.0}, p95 {:.0})",
            seg.name,
            total - args.warmup,
            args.warmup,
            settle_ms,
            percentile(&cpu_ms[args.warmup..], 50.0),
            percentile(&cpu_ms[args.warmup..], 95.0),
            percentile(&cpu_ms[args.warmup..], 100.0),
            percentile(&interval_ms, 50.0),
            percentile(&interval_ms, 95.0),
            fmt_opt(&gpu_total_ms, 50.0),
            fmt_opt(&gpu_total_ms, 95.0),
            percentile(&drawn_objects, 50.0),
            percentile(&drawn_objects, 95.0),
            percentile(&drawn_blend, 50.0),
            percentile(&drawn_tris, 50.0),
            percentile(&drawn_tris, 95.0),
            percentile(&model_parts, 50.0),
            percentile(&model_tris, 50.0),
            percentile(&standins, 50.0),
            percentile(&kit_objs, 50.0),
            percentile(&kit_objs, 95.0),
            percentile(&kit_tris, 50.0),
            percentile(&kit_tris, 95.0),
        );
        // the far layer after the path, and its last frame's counters (entries drawn, triangles, entries in the frustum, entries occluded).
        let mut report = report;
        report["hlod_after"] = json!(models.hlod.as_ref().map(|h| h.stats_json()));
        report["hlod_last_frame"] = json!(renderer.hlod.get().map(|h| h.readback(&ctx.queue)));
        seg_reports.push(report);
    }

    let st = &world.scene.stats;
    let info = &ctx.info;
    let set = &world.authored;
    let alloc = ctx.device.generate_allocator_report();
    let b = renderer.bytes;
    let ms = models.stats.clone();
    let kit_set = world.kit.as_ref().map(|k| {
        json!({
            "kit_buildings": k.buildings, "excluded_authored": k.excluded, "tiles": k.tiles.len(), "tile_m": eo_render::kit::KIT_TILE_M,
            "palette": k.palette.len(), "palette_base_material": k.palette_base,
            "lod_radii_m (near enter, near exit, mid enter, mid exit)": eo_render::kit::DEFAULT_KIT_RADII,
            "stream_radii_m": { "near_load": args.kit_radii.near_load, "near_drop": args.kit_radii.near_drop, "mid_load": args.kit_radii.mid_load, "mid_drop": args.kit_radii.mid_drop },
            "arena_capacity": { "vertices": args.kit_vertices, "indices": args.kit_indices, "objects": args.kit_objects },
            "split_materials": args.kit_split_materials,
            "threads": args.kit_threads,
            "at_first_path": kit_at_start.as_ref().map(|(s, a)| json!({
                "resident_near_packs": s.resident_near, "resident_mid_packs": s.resident_mid, "resident_triangles": s.resident_triangles,
                "resident_vertex_bytes": s.resident_vertex_bytes, "resident_index_bytes": s.resident_index_bytes,
                "packs_from_cache": s.packs_from_cache, "packs_cooked": s.packs_cooked, "cooked_buildings": s.cooked_buildings,
                "unknown_materials": s.unknown_materials, "arena_vertices_peak": a.1, "arena_indices_peak": a.3, "arena_objects_peak": a.5,
                "city_ready_at_ms": s.city_ready_at_ms,
            })),
            "after_each_path": kit_after_paths,
        })
    });
    let rss = process_memory();
    // counted submissions, the driver's spikes seen, those taken in a wait, tiny submissions
    let pacer = eo_render::pace::stats();
    let pacer_json = json!({"on": args.perf.submit_pacing, "submissions": pacer.submissions, "spikes": pacer.spikes, "taken_in_wait": pacer.prepaid, "tiny_submissions": pacer.tiny});
    let mut out = json!({
        "caveat": CAVEAT,
        "adapter": { "name": info.name, "backend": format!("{:?}", info.backend), "device_type": format!("{:?}", info.device_type), "driver": info.driver, "driver_info": info.driver_info },
        "features": {
            "multi_draw_indirect_count_available": ctx.adapter_features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
            "multi_draw_indirect_count_used": renderer.compact,
            "indirect_first_instance": ctx.first_instance,
            "texture_arrays_used": true,
            "timestamp_query": ctx.timestamps,
            "wgpu_indirect_call_validation": args.indirect_validation,
            "timestamp_period_ns": period_ns,
        },
        "settings": {
            "width": w, "height": h, "msaa": args.msaa, "vsync": false, "present": false,
            "fov_y_deg": eo_render::camera::GAME_FOV_Y_DEG, "near_m": eo_render::camera::GAME_NEAR_M, "far_m": args.far,
            "label": args.label, "object_multiplier": args.multiply, "models": args.models, "kit": args.kit, "terrain_lod": args.terrain_lod,
            "terrain_lod_distance_m": eo_render::TERRAIN_LOD_DISTANCE_M, "hlod_enter_exit_m": [renderer.hlod_enter_m, renderer.hlod_exit_m],
            "cache_dir": crate::cache_dir(args).map(|p| p.display().to_string()), "stream_threads": args.stream_threads,
            "upload_budget_bytes_per_frame": args.upload_budget,
        },
        "look": {
            "time_of_day_h": renderer.look.time_of_day(),
            "sun_altitude_deg": renderer.look.frame().model.sun.altitude_deg,
            "exposure": renderer.look.frame().exposure,
            "fog_near_far_m": [renderer.look.frame().aerial.near, renderer.look.frame().aerial.far],
            "shadow_map": renderer.look.options.shadow_map,
            "shadows": renderer.look.options.shadows,
            "cascades_far_m_and_texel_m": renderer.look.cascade_summary(),
            "hdr_target_bytes": renderer.look.hdr_bytes(),
        },
        "loaded": {
            "static_objects": st.objects, "objects_in_gpu_table": renderer.object_count, "static_triangles": st.triangles, "static_vertices": st.vertices,
            "buildings": st.buildings, "building_triangles": st.building_triangles, "footprints_standing_in_for_models": st.authored_footprints,
            "footprints_as_kit_far_boxes": st.kit_boxes, "terrain_tiles": st.terrain_tiles, "terrain_triangles_full": st.terrain_triangles,
            "terrain_triangles_coarse": st.terrain_coarse_triangles, "road_objects": st.road_objects, "road_triangles": st.road_triangles,
        },
        "authored": {
            "manifest_entries": set.manifest_entries, "placed_and_drawn_from_model": set.buildings.len(),
            "skipped": set.skipped.iter().map(|(id, why)| json!({"id": id, "why": why})).collect::<Vec<_>>(),
            "model_parts_reserved": set.object_capacity, "model_triangles_reserved": set.triangles_reserved, "model_vertices_reserved": set.vertex_capacity,
            "resident": ms.resident, "from_cache": ms.from_cache, "cooked_from_glb": ms.cooked,
            "failed": ms.failed.iter().map(|(id, e)| json!({"id": id, "error": e})).collect::<Vec<_>>(),
            "triangles_resident": ms.triangles_resident, "worker_ms_sum": ms.worker_ms, "materials_resident": renderer.material_count,
            "kit_materials": set.materials.kit_materials, "factor_materials": set.materials.factor_materials, "texture_layers": renderer.texture_layers,
            "texture_res": renderer.texture_res, "kit_map_sources": set.materials.layers,
            "kit_bake_deviations": set.materials.deviations.iter().map(|(n, w)| json!({"material": n, "why": w})).collect::<Vec<_>>(),
        },
        "kit": kit_set,
        "hlod": models.hlod.as_ref().map(|h| h.stats_json()), // hlod
        "hlod_gpu_bytes": renderer.hlod.get().map(|h| serde_json::json!({ "vertex": h.bytes[0], "index": h.bytes[1], "pages": h.bytes[2], "tables": h.bytes[3] })), // hlod
        "props": props.report(renderer.props.get().map(|p| p.bytes)), // props
        "cc0": models.cc0.as_ref().map(|c| c.report()), // cc0
        "streaming": {
            "bytes_uploaded_total": ms.bytes_uploaded, "largest_upload_in_one_frame_bytes": ms.max_pump_bytes,
            "longest_render_thread_pump_ms": ms.max_pump_ms, "upload_budget_bytes_per_frame": args.upload_budget,
            // the upload stager (chunks mapped and ready, chunks, times it grew, bytes staged)
            "stager": models.uploads.stager_stats().map(|(ready, n, grew, bytes)| json!({"chunks_ready": ready, "chunks": n, "grew": grew, "bytes_staged": bytes, "on": args.perf.upload_ring})),
            // counted submissions, the driver's spikes seen, those taken in a wait, tiny submissions
            "submission_pacer": pacer_json,
        },
        "vram_bytes": {
            "vertex": b.vertex, "index": b.index, "objects": b.objects, "draw_lists_and_counters": b.draws,
            "materials": b.materials, "lod_groups": b.groups, "textures": b.textures, "total_allocated_by_renderer": b.total(),
            // the pyramid (made at the first frame) and the per-object flags (also in draw_lists_and_counters)
            "occlusion_pyramid_and_flags": renderer.occlusion.bytes(),
            // buffers, the cloud sheet and noise (in `textures` above), and the
            // size-dependent targets (volume, histories, velocity, bloom), not counted above.
            "sky_fixed": renderer.look.sky.bytes,
            "sky_targets": renderer.look.sky.target_bytes(),
            "props": renderer.props.get().map(|p| p.bytes), // props
            "agents": renderer.agents.get().map(|a| a.bytes), // reported beside
            // (in `textures` above) the material arrays, the decal arrays, the tables + street decal grid and records.
            "cc0_material_arrays": renderer.cc0.bytes_by_part().0,
            "cc0_decal_arrays": renderer.cc0.bytes_by_part().1,
            "cc0_tables_and_decal_grid": renderer.cc0.bytes_by_part().2,
            "wgpu_allocator_total_allocated": alloc.as_ref().map(|a| a.total_allocated_bytes),
            "wgpu_allocator_total_reserved": alloc.as_ref().map(|a| a.total_reserved_bytes),
        },
        "system_memory_bytes": { "working_set": rss.0, "peak_working_set": rss.1, "private_bytes": rss.2 },
        "load_ms": {
            "city_json": world.load_ms.city_ms, "elevation": world.load_ms.elevation_ms, "land_cover": world.load_ms.land_cover_ms,
            "authored_prepare (manifest, placement, GLB headers)": world.load_ms.authored_prepare_ms,
            "kit_prepare (groups, tiles, palette)": world.load_ms.kit_prepare_ms,
            "scene_build": world.load_ms.scene_build_ms, "process_start_to_data_and_scene_ready": load_data_ms,
            "process_start_to_first_frame": first_frame_ms, "kit_bake_or_cache_read": ms.kit_bake_ms,
            "process_start_to_kit_maps_on_gpu": ms.kit_ready_at_ms, "process_start_to_all_models_resident": ms.all_resident_at_ms,
            "process_start_to_first_path_settled": streamed_ms,
        },
        "segments": seg_reports,
        "budget_check": { "budget_ms": budget_ms, "enforced_on": STANDARD_SEGMENTS, "rule": "median frame total (CPU pump+encode+submit + GPU) under the budget", "failures": budget_failures },
        // every path's worst frame against the budget (enforced with --check-max).
        "max_check": { "budget_ms": budget_ms, "enforced": args.perf.check_max, "failures": max_failures },
    });
    println!(
        "\nadapter: {} ({:?})\nVRAM (renderer allocations): {:.1} MB (vertex {:.1}, index {:.1}, objects {:.1}, textures {:.1}); allocator report: {}\nsystem memory: working set {:.0} MB (peak {:.0} MB)\nstreaming: {:.1} MB uploaded, largest frame {:.2} MB (budget {:.2} MB), longest pump {:.2} ms\nload: first frame {:.0} ms; first path settled {:.0} ms\n{}",
        info.name,
        info.backend,
        b.total() as f64 / 1e6,
        b.vertex as f64 / 1e6,
        b.index as f64 / 1e6,
        b.objects as f64 / 1e6,
        b.textures as f64 / 1e6,
        alloc.as_ref().map(|a| format!("{:.1} MB allocated, {:.1} MB reserved", a.total_allocated_bytes as f64 / 1e6, a.total_reserved_bytes as f64 / 1e6)).unwrap_or("n/a".into()),
        rss.0 as f64 / 1e6,
        rss.1 as f64 / 1e6,
        ms.bytes_uploaded as f64 / 1e6,
        ms.max_pump_bytes as f64 / 1e6,
        args.upload_budget as f64 / 1e6,
        ms.max_pump_ms,
        first_frame_ms,
        streamed_ms,
        CAVEAT
    );
    let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let label = args.label.as_deref().unwrap_or("default");
    let path = dir.join(format!("bench-{label}-{stamp}.json"));
    // the switch and its numbers, the arenas (outside the json! above: its recursion limit).
    out["settings"]["view_detail"] = json!({
        "on": args.perf.view_detail.enabled, "margin_deg": args.perf.view_detail.margin_deg, "predict_s": args.perf.view_detail.predict_s,
        "hold_s": args.perf.view_detail.hold_s, "stream_all_paths": args.perf.stream_all_paths, "full_arenas": args.perf.full_arenas,
        "model_load_drop_ring_m": [crate::models::MODEL_LOAD_M, crate::models::MODEL_DROP_M, crate::models::MODEL_RING_M],
        "kit_arena": models.kit.as_ref().map(|k| [k.arena.vertex_capacity, k.arena.index_capacity, k.arena.object_capacity]),
        "kit_near_part": models.kit.as_ref().and_then(|k| k.near_pool_capacity()),
        "street_arena": models.street.as_ref().map(|s| [s.arena.vertex_capacity, s.arena.index_capacity, s.arena.object_capacity]),
        "model_arena": models.model_arena_use(),
        "model_slots_all": [set.vertex_capacity, set.index_capacity],
    });
    std::fs::write(&path, serde_json::to_string_pretty(&out)?)?;
    println!("wrote {}", path.display());
    audio.final_report(); // audio
    if budget_failures.is_empty() {
        println!("budget check: PASS (median frame total under {budget_ms} ms on {STANDARD_SEGMENTS:?})");
    } else {
        println!("budget check: FAIL {budget_failures:?}");
    }
    if ms.max_pump_bytes > args.upload_budget {
        println!("upload budget: largest frame {} B over the {} B budget", ms.max_pump_bytes, args.upload_budget);
        budget_failures.push(format!("upload {} B in one frame over the {} B budget", ms.max_pump_bytes, args.upload_budget));
    }
    if args.capture {
        // the promenade, a pier, the skyline across the bay, the bay from the air, Ocean Beach
        // (EO_CAPTURE_ONLY=water: only these).
        if let Some(w) = renderer.water.get() {
            w.set_time(20.0);
            w.set_always(true);
        }
        // the bridges, the shore edges, the open spaces (EO_CAPTURE_ONLY=landmarks: only these).
        let only = std::env::var("EO_CAPTURE_ONLY").ok();
        if only.as_deref().is_none_or(|v| v == "landmarks") {
            for (name, cam, what) in landmarks_bench::capture_cameras(&world, aspect, args.far) {
                settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
                crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam);
                let p = dir.join(format!("capture-landmarks-{label}-{name}.png"));
                capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
                println!("wrote {} (eye {:.1} {:.2} {:.1}: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
            }
            if only.is_some() {
                return Ok(());
            }
        }
        if only.as_deref() == Some("water") {
            renderer.look.sky.set_clock(args.look.sky.world_time_s, 0.0);
        }
        for (name, cam, what) in water_bench::capture_cameras(&world, aspect, args.far) {
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam);
            let p = dir.join(format!("capture-water-{label}-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1}: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
        // the bay from the city_overview path's first camera.
        if let Some(seg) = segs.iter().find(|s| s.name == "city_overview") {
            let cam = (seg.camera)(0, args.warmup + seg.frames);
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            let p = dir.join(format!("capture-water-{label}-city_overview.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1}: the city_overview path's first frame)", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
        if std::env::var("EO_CAPTURE_ONLY").is_ok_and(|v| v == "water") {
            return Ok(());
        }
        for seg in &segs {
            let total = args.warmup + seg.frames;
            let frames: Vec<usize> = match seg.name {
                "authored_street" | "kit_street" => vec![0, total / 6, total / 3, total / 2, 2 * total / 3, 5 * total / 6],
                "drive" => vec![0, total / 2, total - 1],
                _ => vec![0, total / 4],
            };
            // the night, rain and fog paths under the hour and weather they are measured in
            // (the frames were captured at the default noon, clear, before).
            let sky = sky_shots::enter(&renderer, args, seg.name);
            for i in frames {
                // a frame looking into a wall or standing in a building moves on along the path.
                let (i, moved) = clear_path_frame(seg, i, total);
                if let Some(m) = moved {
                    println!("capture: {}: {m}", seg.name);
                }
                let cam = (seg.camera)(i, total);
                settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
                crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam); // props
                let p = dir.join(format!("capture-{label}-{}-frame{i}.png", seg.name));
                capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
                println!("wrote {}", p.display());
            }
            sky_shots::leave(&renderer, sky);
        }
        // kit_street from near, mid and far: evidence the frame contains kit
        // buildings at each level (rung 0, rung 2, boxes), not a quality judgement.
        let ksp = kit_street();
        let ks = Vec3::new(ksp[0][0], 0.0, ksp[0][1]);
        let kdir = (Vec3::new(ksp[1][0], 0.0, ksp[1][1]) - ks).normalize();
        let mid_pt = ks + kdir * 75.0 * gsf();
        let g = world.elevation.ground(mid_pt.x as f64, mid_pt.z as f64) as f32;
        let side = Vec3::new(-kdir.z, 0.0, kdir.x);
        for (name, back, up) in [("near", 0.0f32, EYE_HEIGHT_M), ("mid", 260.0, 60.0), ("far", 900.0, 160.0)] {
            let eye = if back == 0.0 {
                // On the street 40 m along, looking along it at eye height.
                let p = ks + kdir * 40.0;
                Vec3::new(p.x, world.elevation.ground(p.x as f64, p.z as f64) as f32 + up, p.z)
            } else {
                mid_pt + side * back + Vec3::new(0.0, g + up, 0.0)
            };
            let to = if back == 0.0 { kdir + Vec3::new(0.0, 0.03, 0.0) } else { Vec3::new(mid_pt.x, g + 8.0, mid_pt.z) - eye };
            let cam = Camera { far: args.far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) };
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            let p = dir.join(format!("capture-{label}-kit_street-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {}", p.display());
        }
        // close views of walls, windows and a shopfront (kit and
        // Blender-built), for matched before/after captures of the facade pass.
        for (name, cam) in facade_cameras(&segs, &world, args.far) {
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            let p = dir.join(format!("capture-{label}-facade-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
        // Slice 4: the street and grounding views.
        for (name, cam, what) in street_bench::s4_cameras(&world, aspect, args.far) {
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            let p = dir.join(format!("capture-s4-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1}: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
        // a signalised junction, a tree-lined street, a kerb with parked cars.
        for (name, cam, what) in crate::props::capture_cameras(&world, aspect, args.far) {
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam);
            let p = dir.join(format!("capture-{label}-props-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1}: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
        // a wall per finish class, a shop pane, the bank step, the hill.
        for (name, cam, what) in fix3_shots::cameras(&world, aspect, args.far) {
            settle_p(&ctx, &renderer, &world, &mut models, Some(&mut props), &cam, 600.0)?;
            crate::props::sync_parked(&ctx, &renderer, &mut props, &mut agents, &cam);
            let p = dir.join(format!("capture-{label}-fix3-{name}.png"));
            capture(&ctx, &renderer, &color, attach, resolve, &targets.depth, &cam, &p)?;
            println!("wrote {} (eye {:.1} {:.2} {:.1}: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
        }
    }
    if max_failures.is_empty() {
        println!("max check: PASS (every path's worst frame under {budget_ms} ms)");
    } else {
        println!("max check: FAIL {max_failures:?}");
    }
    if args.check_budget && !budget_failures.is_empty() {
        anyhow::bail!("frame budget exceeded: {budget_failures:?}");
    }
    if args.perf.check_max && !max_failures.is_empty() {
        anyhow::bail!("worst frame over the budget: {max_failures:?}");
    }
    Ok(())
}

/// Working set, peak working set, private bytes of this process (Windows; zeros elsewhere).
fn process_memory() -> (u64, u64, u64) {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
            private_usage: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
        }
        let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
        // SAFETY: plain Win32 call with a correctly sized, writable struct.
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        if ok != 0 {
            return (c.working_set_size as u64, c.peak_working_set_size as u64, c.private_usage as u64);
        }
    }
    (0, 0, 0)
}

/// Render one camera into the offscreen target and save it as PNG. Used only
/// to confirm the bench frames contain the city; not a visual-quality claim.
#[allow(clippy::too_many_arguments)]
/// the street world (its building footprints), set once the bench's
/// world is loaded, so every capture can say when its eye stands inside a
/// building (a pose that drifted into a building when the plan changed).
static CAPTURE_BUILDINGS: std::sync::OnceLock<std::sync::Arc<eo_street::StreetWorld>> = std::sync::OnceLock::new();

/// an eye higher than this above the ground is an aerial view; a
/// footprint under it is a roof, not a wall in front of it.
const CAPTURE_STREET_LEVEL_M: f32 = 30.0;
/// a wall nearer than this straight ahead fills the frame (warned).
const CAPTURE_WALL_WARN_M: f32 = 3.0;
/// a path frame moves on until no wall is nearer than this straight ahead.
const CAPTURE_WALL_CLEAR_M: f32 = 6.0;
/// a facade close-up stands this far in front of the frontage when
/// its fixed step across the street would put it in or against the building.
const FACADE_STANDOFF_M: f32 = 5.0;

/// the street world and whether `eye` is at street level there.
fn capture_street(eye: Vec3) -> Option<&'static eo_street::StreetWorld> {
    let w = CAPTURE_BUILDINGS.get()?;
    let ground = w.terrain.grid.ground(eye.x as f64, eye.z as f64) as f32;
    (eye.y - ground <= CAPTURE_STREET_LEVEL_M).then_some(&**w)
}

/// the footprint a street-level eye stands in, if any.
fn eye_inside_building(eye: Vec3) -> Option<u32> {
    capture_street(eye)?.buildings.at(eye.x as f64, eye.z as f64)
}

/// metres from a street-level eye to the first building footprint
/// straight ahead along `dir` (level), if one starts within `max` metres.
fn wall_ahead(eye: Vec3, dir: Vec3, max: f32) -> Option<f32> {
    let w = capture_street(eye)?;
    let f = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    if f == Vec3::ZERO {
        return None;
    }
    let mut d = 0.0f32;
    while d <= max {
        let p = eye + f * d;
        if w.buildings.contains(p.x as f64, p.z as f64) {
            return Some(d);
        }
        d += 0.1;
    }
    None
}

/// why a camera would not show the street (inside a building, or a
/// wall within `clear` metres straight ahead), or None when its view is clear.
fn blocked_view(cam: &Camera, clear: f32) -> Option<String> {
    if let Some(b) = eye_inside_building(cam.eye) {
        return Some(format!("eye inside building footprint {b}"));
    }
    wall_ahead(cam.eye, cam.forward(), clear).map(|d| format!("a wall {d:.1} m ahead"))
}

/// the frame to capture for a path's nominal frame `i`: `i` itself,
/// or, when its view is blocked, the nearest frame of the path (either way, at
/// most a quarter of the path off) whose view is clear, with the reason it moved.
/// (street_walk turns a full circle from a spot 1.7 m off a wall, so its
/// quarter frame has always looked into that wall.)
fn clear_path_frame(seg: &Segment, i: usize, total: usize) -> (usize, Option<String>) {
    let Some(why) = blocked_view(&(seg.camera)(i, total), CAPTURE_WALL_CLEAR_M) else {
        return (i, None);
    };
    let reach = total / 4;
    let clear = |j: usize| j < total && blocked_view(&(seg.camera)(j, total), CAPTURE_WALL_CLEAR_M).is_none();
    let found = (1..=reach).find_map(|k| [Some(i + k), i.checked_sub(k)].into_iter().flatten().find(|&j| clear(j)));
    match found {
        Some(j) => (j, Some(format!("frame {i} moved to {j}: {why}"))),
        None => (i, Some(format!("frame {i} kept, no clear view within {reach} frames either way: {why}"))),
    }
}

fn capture(
    ctx: &GpuContext,
    renderer: &Renderer,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    depth: &wgpu::TextureView,
    cam: &Camera,
    path: &std::path::Path,
) -> Result<()> {
    if let Some(why) = blocked_view(cam, CAPTURE_WALL_WARN_M) {
        // a frame from inside a building, or against a wall, shows its back faces or a blank wall, not the street.
        println!("capture: BLOCKED VIEW: {why} at eye ({:.1}, {:.1}, {:.1}) for {}", cam.eye.x, cam.eye.y, cam.eye.z, path.display());
    }
    let (w, h) = (color.width(), color.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture"),
        size: row as u64 * h as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // Two frames: the first settles the LOD hysteresis for this camera.
    // twenty: the scene probe captures at most one cube face a frame
    // (six faces and the levels after a move of more than 16 m; a face waits
    // on a frame that refreshes a middle or far cascade), so with two frames
    // every capture reflected the probe of the previous camera, often a
    // kilometre away; the occlusion/bounce history settles with it.
    for _ in 0..20 {
        renderer.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        renderer.encode(&mut enc, attach, resolve, depth, None);
        eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
    }
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
    let bytes = read_mapped(ctx, &buf)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + w as usize * 4]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

/// one measured frame's render-thread record.
struct FrameRecord {
    trace: hitch::FrameTrace,
    /// The render thread's own CPU time over the frame's CPU part.
    on_cpu_ms: f64,
    /// Waiting for the frame two back (not part of the frame total).
    wait_ms: f64,
    /// Allocator blocks and reserved bytes before the frame (EO_HITCH_ALLOC=1).
    alloc: Option<(u32, u64)>,
    /// the submission spike's prepay before the wait (what it did,
    /// its time, the part charged to the frame).
    prepay: (eo_render::pace::Prepay, f64, f64),
}

/// a prepay followed by a wait for the GPU at least this long (ms)
/// ran in time the render thread would have spent waiting.
const PREPAY_FREE_WAIT_MS: f64 = eo_render::pace::PREPAY_FREE_WAIT_MS; // one value for every loop

fn fmt_opt(v: &[f64], p: f64) -> String {
    if v.is_empty() { "n/a".into() } else { format!("{:.3}", percentile(v, p)) }
}
