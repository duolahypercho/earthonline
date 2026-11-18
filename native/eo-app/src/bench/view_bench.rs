//! the bench's side of the view-gated best-quality layer.
//!
//! - `turn_segments`: two turn-around paths (stand on a dense street, yaw
//!   180 degrees in 0.25 s, hold, turn back, hold past the hysteresis, turn
//!   again): `turn_around` on Castro Street among kit buildings and
//!   `turn_around_authored` on Folsom Street among the Blender-built ones.
//!   Paced like the drive (the camera follows the wall clock), so the
//!   streamers get real time.
//! - the view clock the bench gives the renderer: camera time on a path (its
//!   frame index at 60 Hz), wall time while settling.
//! - `PathTrack`: per frame, what is on screen (inside the true frustum) and
//!   drawn at a lower level than its distance zone wants, and how long each
//!   stays that way; resident memory, bytes uploaded and worker time per path.

use super::{EYE_HEIGHT_M, FRAME_PERIOD_S, Segment, yaw_of};
use crate::World;
use crate::models::{ModelRuntime, ModelState};
use eo_render::camera::{Camera, aabb_visible, frustum_planes};
use glam::Vec3;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// The view clock (seconds) the bench hands the renderer.
static CLOCK: Mutex<f64> = Mutex::new(0.0);

pub fn clock() -> f64 {
    *CLOCK.lock().unwrap()
}

pub fn set_clock(t: f64) {
    *CLOCK.lock().unwrap() = t;
}

/// Settling: the clock moves with the wall.
pub fn advance(dt: f64) -> f64 {
    let mut c = CLOCK.lock().unwrap();
    *c += dt.max(0.0);
    *c
}

/// The turn timeline, seconds after the measured frames start: hold, turn
/// 180 degrees in TURN_S, hold (inside the hysteresis), turn back, hold past
/// the hysteresis, turn again, hold.
pub const TURN_S: f32 = 0.25;
const TURNS: [(f32, f32); 3] = [(1.0, 1.0), (3.25, -1.0), (7.5, 1.0)];

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Yaw offset (radians) at `t` seconds after the measured start.
pub fn turn_offset(t: f32) -> f32 {
    let mut a = 0.0;
    for (start, dir) in TURNS {
        a += dir * std::f32::consts::PI * smoothstep((t - start) / TURN_S);
    }
    a
}

fn eye_on_ground(world: &World, p: Vec3) -> Vec3 {
    let y = match &world.street {
        Some(s) => s.world.surface_at(p.x as f64, p.z as f64, None) as f32,
        None => world.elevation.ground(p.x as f64, p.z as f64) as f32,
    };
    Vec3::new(p.x, y + EYE_HEIGHT_M, p.z)
}

/// The two turn-around paths (`frames` measured frames each; 720 is 12 s).
pub fn turn_segments(world: &World, frames: usize, warmup: usize, aspect: f32, far: f32) -> Vec<Segment> {
    let mut out = vec![];
    // Castro Street, 45 m into the kit walk, looking 45 degrees across the street.
    // the same places on the scaled ground plan (positions and distances along the street scaled).
    let gs = super::gsf();
    let ksp = super::kit_street();
    let ks = Vec3::new(ksp[0][0], 0.0, ksp[0][1]);
    let kdir = (Vec3::new(ksp[1][0], 0.0, ksp[1][1]) - ks).normalize();
    // Folsom Street, 40 m from the spawn toward the side with more Blender buildings.
    let heading = super::folsom_heading(world);
    let start = Vec3::new(super::spawn()[0], 0.0, super::spawn()[1]);
    let (east, west) = (super::authored_along(world, start, heading, 150.0 * gs, 45.0), super::authored_along(world, start, -heading, 150.0 * gs, 45.0));
    let fdir = if west > east { -heading } else { heading };
    for (name, at, dir, where_) in [
        ("turn_around", ks + kdir * 45.0 * gs, kdir, "Castro Street (kit buildings; the nearest Blender-built building is 478 m away)"),
        ("turn_around_authored", start + fdir * 40.0 * gs, fdir, "Folsom Street by the spawn (Blender-built buildings on both sides)"),
    ] {
        let eye = eye_on_ground(world, at);
        let yaw0 = yaw_of(dir) + 45f32.to_radians();
        out.push(Segment {
            name,
            description: format!(
                "viewdetail: eye {EYE_HEIGHT_M} m above the drawn street at ({:.0}, {:.0}) on {where_}, looking 45 degrees off the street's direction; after 1 s the view turns 180 degrees in {TURN_S} s (smoothstep), holds 2 s (inside the 2.5 s hysteresis), turns back, holds 4 s (past it), turns again and holds; paced like the drive (the camera follows the wall clock)",
                eye.x, eye.z
            ),
            camera: Box::new(move |i, _n| {
                let t = (i as f32 - warmup as f32) * FRAME_PERIOD_S as f32;
                Camera { far, ..Camera::new(eye, yaw0 + turn_offset(t), 0.02, aspect) }
            }),
            frames,
            paced: true,
            streams: true,
        });
    }
    out
}

/// One kind of degraded thing: how long each stays degraded.
#[derive(Default)]
struct Episodes<K> {
    open: HashMap<K, f64>,
    closed_ms: Vec<f64>,
    censored_ms: Vec<f64>,
    degraded_frames: u64,
    frames_with_any: u64,
    max_at_once: usize,
}

impl<K: Eq + std::hash::Hash + Copy> Episodes<K> {
    fn frame(&mut self, t_ms: f64, now: &HashSet<K>) {
        let ended: Vec<K> = self.open.keys().filter(|k| !now.contains(k)).copied().collect();
        for k in ended {
            let s = self.open.remove(&k).unwrap();
            self.closed_ms.push(t_ms - s);
        }
        for k in now {
            self.open.entry(*k).or_insert(t_ms);
        }
        self.degraded_frames += now.len() as u64;
        if !now.is_empty() {
            self.frames_with_any += 1;
        }
        self.max_at_once = self.max_at_once.max(now.len());
    }

    fn finish(&mut self, t_end_ms: f64) {
        for (_, s) in self.open.drain() {
            self.censored_ms.push(t_end_ms - s);
        }
    }

    fn report(&self) -> serde_json::Value {
        let all: Vec<f64> = self.closed_ms.iter().chain(&self.censored_ms).copied().collect();
        json!({
            "episodes": self.closed_ms.len(),
            "still_degraded_at_path_end": self.censored_ms.len(),
            "duration_ms (episodes, and those still open at the end, from their start)": {
                "max": super::percentile(&all, 100.0), "p95": super::percentile(&all, 95.0), "median": super::percentile(&all, 50.0),
            },
            "degraded_object_frames": self.degraded_frames,
            "frames_with_any": self.frames_with_any,
            "most_at_once": self.max_at_once,
        })
    }

    fn line(&self) -> String {
        let all: Vec<f64> = self.closed_ms.iter().chain(&self.censored_ms).copied().collect();
        if all.is_empty() {
            return "none".into();
        }
        format!(
            "{} episodes (+{} open at the end), max {:.0} ms, p95 {:.0} ms, {} frames with any, most at once {}",
            self.closed_ms.len(),
            self.censored_ms.len(),
            super::percentile(&all, 100.0),
            super::percentile(&all, 95.0),
            self.frames_with_any,
            self.max_at_once
        )
    }
}

#[derive(Default, Clone, Copy)]
struct Mem {
    kit: u64,
    street: u64,
    model: u64,
    kit_near: usize,
    kit_mid: usize,
    street_tiles: usize,
    models: usize,
}

fn mem_now(models: &ModelRuntime) -> Mem {
    let (mut m, r) = (Mem::default(), &models.residency);
    if let Some(k) = &models.kit {
        m.kit = k.stats.resident_vertex_bytes + k.stats.resident_index_bytes;
        m.kit_near = k.stats.resident_near;
        m.kit_mid = k.stats.resident_mid;
    }
    if let Some(s) = &models.street {
        m.street = s.stats.resident_vertex_bytes + s.stats.resident_index_bytes;
        m.street_tiles = s.stats.resident;
    }
    m.model = r.resident_vertex_bytes + r.resident_index_bytes;
    m.models = r.resident;
    m
}

/// Worker milliseconds so far: kit packs, street tiles, models, props tiles.
fn work_now(models: &ModelRuntime, props: &crate::props::PropsRuntime) -> [f64; 4] {
    [
        models.kit.as_ref().map(|k| k.stats.worker_ms).unwrap_or(0.0),
        models.street.as_ref().map(|s| s.stats.worker_ms).unwrap_or(0.0),
        models.stats.worker_ms,
        props.stats.plan_ms_total,
    ]
}

/// Kit near zone and the mid radius (to the bounds), the Blender HLOD enter
/// radius, the street near radius: the zones the GPU's LOD pass draws by.
const KIT_NEAR_M: f32 = eo_render::kit::DEFAULT_KIT_RADII[0];
const KIT_MID_M: f32 = eo_render::kit::DEFAULT_KIT_RADII[2];
const STREET_NEAR_M: f32 = eo_render::street::DEFAULT_STREET_RADII[0];

/// One path's view-detail record.
pub struct PathTrack {
    kit: Episodes<u32>,
    kit_box: Episodes<u32>,
    model: Episodes<usize>,
    model_ring: Episodes<usize>,
    street: Episodes<u32>,
    peak: Mem,
    start: Mem,
    bytes_settle: u64,
    bytes_path: u64,
    work0: [f64; 4],
    work1: [f64; 4],
    last_t_ms: f64,
    frames: usize,
    gated_now: [usize; 3],
}

impl PathTrack {
    /// Before the path's settle.
    pub fn begin(models: &ModelRuntime, props: &crate::props::PropsRuntime) -> Self {
        Self {
            kit: Default::default(),
            kit_box: Default::default(),
            model: Default::default(),
            model_ring: Default::default(),
            street: Default::default(),
            peak: Mem::default(),
            start: Mem::default(),
            bytes_settle: models.stats.bytes_uploaded,
            bytes_path: 0,
            work0: work_now(models, props),
            work1: [0.0; 4],
            last_t_ms: 0.0,
            frames: 0,
            gated_now: [0; 3],
        }
    }

    /// After the settle, before the first frame.
    pub fn settled(&mut self, models: &ModelRuntime, props: &crate::props::PropsRuntime) {
        self.bytes_settle = models.stats.bytes_uploaded - self.bytes_settle;
        self.start = mem_now(models);
        self.peak = self.start;
        self.work1 = work_now(models, props);
    }

    /// One frame: `bytes` its streaming wrote; `measured` once past the warm-up.
    pub fn frame(&mut self, world: &World, models: &ModelRuntime, cam: &Camera, t_ms: f64, bytes: u64, measured: bool) {
        self.bytes_path += bytes;
        let m = mem_now(models);
        let p = &mut self.peak;
        p.kit = p.kit.max(m.kit);
        p.street = p.street.max(m.street);
        p.model = p.model.max(m.model);
        p.kit_near = p.kit_near.max(m.kit_near);
        p.kit_mid = p.kit_mid.max(m.kit_mid);
        p.street_tiles = p.street_tiles.max(m.street_tiles);
        p.models = p.models.max(m.models);
        if !measured {
            return;
        }
        self.frames += 1;
        self.last_t_ms = t_ms;
        let planes = frustum_planes(cam.view_proj());
        let eye = cam.eye;
        // Kit: within the near zone and on screen without its near pack (rung 2
        // or the box drawn); within the mid radius drawn as the box.
        let (mut kit_now, mut box_now) = (HashSet::new(), HashSet::new());
        if let Some(k) = &models.kit {
            let set = &k.set;
            let reach = ((KIT_MID_M as f64 + 128.0) / eo_render::kit::KIT_TILE_M).ceil() as i32;
            let (cx, cz) = ((eye.x as f64 / eo_render::kit::KIT_TILE_M).floor() as i32, (eye.z as f64 / eo_render::kit::KIT_TILE_M).floor() as i32);
            for tz in cz - reach..=cz + reach {
                for tx in cx - reach..=cx + reach {
                    let Some(&t) = set.tile_of_key.get(&(tx, tz)) else { continue };
                    let tile = &set.tiles[t as usize];
                    if set.tile_distance(t as usize, eye) > KIT_MID_M + 64.0 {
                        continue;
                    }
                    for j in 0..tile.buildings.len() {
                        let rel = tile.group_first as usize + j;
                        let g = &set.groups[rel];
                        if g.kind != eo_render::kit::GROUP_KIND_KIT {
                            continue;
                        }
                        let c = Vec3::from(g.center);
                        let d = ((c - eye).length() - g.radius).max(0.0);
                        if d > KIT_MID_M || !aabb_visible(&planes, c - g.radius, c + g.radius) {
                            continue;
                        }
                        let ready = k.group_ready(rel);
                        if d <= KIT_NEAR_M && ready & 1 == 0 {
                            kit_now.insert(rel as u32);
                        }
                        if ready & 3 == 0 {
                            box_now.insert(rel as u32);
                        }
                    }
                }
            }
            self.gated_now[0] = k.near_out_of_view;
        }
        // Blender-built: within the HLOD enter radius and on screen, footprint drawn.
        let (mut model_now, mut ring_now) = (HashSet::new(), HashSet::new());
        let set = &world.authored;
        for (i, b) in set.buildings.iter().enumerate() {
            let g = &set.groups[b.group as usize];
            let c = Vec3::from(g.center);
            let d = (c - eye).length() - g.radius;
            if d > set.hlod_enter_m || !aabb_visible(&planes, c - g.radius, c + g.radius) {
                continue;
            }
            if !matches!(models.model_state.get(i), Some(ModelState::Resident) | Some(ModelState::Failed)) {
                model_now.insert(i);
                if d <= crate::models::MODEL_RING_M {
                    ring_now.insert(i);
                }
            }
        }
        self.gated_now[1] = models.residency.out_of_view;
        // Streets: within the near radius and on screen, far strips drawn.
        let mut street_now = HashSet::new();
        if let Some(s) = &models.street {
            let set = &s.set;
            let reach = ((STREET_NEAR_M as f64 + 128.0) / eo_street::STREET_TILE_M).ceil() as i32;
            let (cx, cz) = eo_street::tile_key(eye.x as f64, eye.z as f64);
            for tz in cz - reach..=cz + reach {
                for tx in cx - reach..=cx + reach {
                    let Some(&t) = set.tile_of_key.get(&(tx, tz)) else { continue };
                    let g = &set.groups[t as usize];
                    if (Vec3::from(g.center) - eye).length() - g.radius > STREET_NEAR_M {
                        continue;
                    }
                    let (mn, mx) = set.tile_box(t as usize);
                    if aabb_visible(&planes, mn, mx) && !s.is_resident(t) {
                        street_now.insert(t);
                    }
                }
            }
            self.gated_now[2] = s.near_out_of_view;
        }
        self.kit.frame(t_ms, &kit_now);
        self.kit_box.frame(t_ms, &box_now);
        self.model.frame(t_ms, &model_now);
        self.model_ring.frame(t_ms, &ring_now);
        self.street.frame(t_ms, &street_now);
    }

    /// After the path's last frame.
    pub fn finish(&mut self, models: &ModelRuntime, props: &crate::props::PropsRuntime, on: bool) -> serde_json::Value {
        let t = self.last_t_ms + FRAME_PERIOD_S * 1e3;
        for e in [&mut self.kit, &mut self.kit_box, &mut self.street] {
            e.finish(t);
        }
        self.model.finish(t);
        self.model_ring.finish(t);
        let end = mem_now(models);
        let w = work_now(models, props);
        let mb = |b: u64| b as f64 / 1e6;
        let mem = |m: &Mem| {
            json!({
                "kit_mb": mb(m.kit), "street_mb": mb(m.street), "models_mb": mb(m.model), "total_mb": mb(m.kit + m.street + m.model),
                "kit_near_packs": m.kit_near, "kit_mid_packs": m.kit_mid, "street_near_tiles": m.street_tiles, "blender_models": m.models,
            })
        };
        let names = ["kit_packs", "street_tiles", "blender_models", "props_tiles"];
        let work = |a: &[f64; 4], b: &[f64; 4]| names.iter().enumerate().map(|(k, n)| (n.to_string(), json!(b[k] - a[k]))).collect::<serde_json::Map<_, _>>();
        println!(
            "    viewdetail: resident at start {:.1} MB, peak {:.1} MB, end {:.1} MB (kit {:.1} / street {:.1} / models {:.1} MB at the end; near packs {} mid {} street tiles {} models {}); uploaded {:.1} MB settling + {:.1} MB on the path; worker ms on the path kit {:.0} street {:.0} models {:.0} props {:.0}",
            mb(self.start.kit + self.start.street + self.start.model),
            mb(self.peak.kit + self.peak.street + self.peak.model),
            mb(end.kit + end.street + end.model),
            mb(end.kit),
            mb(end.street),
            mb(end.model),
            end.kit_near,
            end.kit_mid,
            end.street_tiles,
            end.models,
            mb(self.bytes_settle),
            mb(self.bytes_path),
            w[0] - self.work1[0],
            w[1] - self.work1[1],
            w[2] - self.work1[2],
            w[3] - self.work1[3],
        );
        println!("    pop-in (on screen, lower level than the zone wants): kit within 65 m: {}", self.kit.line());
        println!("      kit within 470 m drawn as the box: {}", self.kit_box.line());
        println!("      Blender models within 320 m as the footprint: {}", self.model.line());
        println!("      Blender models within {} m (the ring, every direction) as the footprint: {}", crate::models::MODEL_RING_M, self.model_ring.line());
        println!("      street tiles within 220 m as the far strips: {}", self.street.line());
        json!({
            "on": on,
            "measured_frames": self.frames,
            "pop_in": {
                "definition": "per measured frame, things inside the camera frustum (not tested for occlusion) within their zone and drawn at a lower level than it wants; an episode runs from the first such frame to the first frame it is not (camera time, 60 Hz frame steps)",
                "kit_within_65m_without_near_pack": self.kit.report(),
                "kit_within_470m_drawn_as_box": self.kit_box.report(),
                "blender_within_320m_as_footprint": self.model.report(),
                "blender_within_ring_150m_as_footprint": self.model_ring.report(),
                "street_within_220m_as_far_strips": self.street.report(),
            },
            "resident": { "at_start": mem(&self.start), "peak": mem(&self.peak), "at_end": mem(&end) },
            "bytes_uploaded": { "settle_before_path": self.bytes_settle, "path_all_frames": self.bytes_path },
            "worker_ms": { "settle_before_path": work(&self.work0, &self.work1), "path": work(&self.work1, &w) },
            "left_out_as_out_of_view_last_frame": { "kit_near_packs": self.gated_now[0], "blender_models": self.gated_now[1], "street_tiles": self.gated_now[2] },
            "models": {
                "uploaded": models.residency.uploaded, "evicted": models.residency.evicted, "cancelled_queued": models.residency.cancelled,
                "dropped_on_arrival": models.residency.dropped_unneeded, "peak_mb": mb(models.residency.peak_bytes),
                "peak_vertices_indices_so_far": [models.residency.peak_vertices, models.residency.peak_indices], "arena_full_arrivals": models.residency.arena_full,
                "arena_capacity_and_peak (vertices, indices)": models.model_arena_use(),
            },
            // The arenas' use so far in the run (peaks since process start), for sizing them.
            "arenas": {
                "kit_used_peak (v, v, i, i, o, o)": models.kit.as_ref().map(|k| k.arena_use()),
                "kit_resident_by_rung_peak (near, mid: vertices, indices, objects)": models.kit.as_ref().map(|k| k.stats.rung_peak),
                "kit_near_part_capacity": models.kit.as_ref().and_then(|k| k.near_pool_capacity()),
                "kit_near_part_use_peak": models.kit.as_ref().and_then(|k| k.near_pool_use()),
                "kit_packs_deferred": models.kit.as_ref().map(|k| k.stats.packs_deferred),
                "street_used_peak (v, v, i, i, o, o)": models.street.as_ref().map(|s| s.arena_use()),
                "street_packs_arena_full": models.street.as_ref().map(|s| s.stats.packs_skipped_arena_full),
            },
        })
    }
}

/// `EO_VD_CAPTURE=seg:frame,...`: capture these path frames mid-path (their
/// own camera, after that frame's streaming), as `capture-vd-<label>-<seg>-f<frame>.png`.
pub fn capture_frames(seg: &str) -> Vec<usize> {
    let mut v: Vec<usize> = std::env::var("EO_VD_CAPTURE")
        .ok()
        .map(|v| {
            v.split(',')
                .filter_map(|p| {
                    let (s, f) = p.split_once(':')?;
                    (s.trim() == seg).then(|| f.trim().parse().ok()).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_unstable();
    v
}
