//! The grade-separated roads' bench paths (`--structures`), and the places
//! the structures capture card (`structures_shots.rs`) looks at:
//!
//! * `freeway_drive`: eye 1.5 m over a built freeway deck near the I-80
//!   approach (the built Route 80 chain nearest 5th Street at Harrison
//!   Street, at least `DRIVE_M` long; else the longest built Route 80
//!   chain; else the longest built motorway deck), in its right lane,
//!   150 m along it over the path, looking ahead with a gentle sweep;
//! * `under_viaduct`: eye 1.7 m on a street under a built deck (where a
//!   built motorway deck crosses 5th Street, else Selby Street under I-280,
//!   else the first built motorway deck over a paved street), looking along
//!   the street and sweeping across the deck overhead.
//!
//! Both are None without structures (or with no built deck), so the path
//! list is the same with or without them.

use super::{EYE_HEIGHT_M, Segment, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use eo_structures::{Chain, ChainKind, RoadClass, StructureWorld};
use glam::Vec3;
use std::sync::Arc;

/// The freeway drive's length and eye height over the deck.
pub const DRIVE_M: f64 = 150.0;
const DRIVE_EYE_M: f32 = 1.5;
/// Half the drive's sweep either side of ahead, and the under-viaduct sweep's.
const DRIVE_SWEEP_DEG: f32 = 12.0;
const UNDER_SWEEP_DEG: f32 = 70.0;

/// A built deck chain drawn by the renderer (not a landmark's, not a tunnel).
pub(super) fn drawn_deck(c: &Chain) -> bool {
    c.is_built() && c.kind == ChainKind::Deck
}

pub(super) fn named(c: &Chain, name: &str) -> bool {
    c.pieces.iter().any(|p| p.name.contains(name))
}

/// The plan junction where streets `a` and `b` meet (the first in plan order).
pub(super) fn junction(world: &World, a: &str, b: &str) -> Option<Vec3> {
    let sw = &world.street.as_ref()?.world;
    let name = |ap: &eo_street::plan::Approach| sw.city.segments[sw.plan.entries[ap.entry].road].street_name.as_str();
    sw.plan.nodes.iter().find(|n| n.approaches.iter().any(|ap| name(ap) == a) && n.approaches.iter().any(|ap| name(ap) == b)).map(|n| Vec3::new(n.position[0] as f32, 0.0, n.position[1] as f32))
}

/// The drawn street surface under (x, z) (the ground without streets).
pub(super) fn surface(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// The nearest point of a named street's segments to `p` and the street's
/// unit direction there.
pub(super) fn street_point(world: &World, name: &str, p: Vec3) -> Option<(Vec3, Vec3)> {
    let sw = &world.street.as_ref()?.world;
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    for s in sw.city.segments.iter().filter(|s| s.street_name == name && !s.separated) {
        for w in s.points.windows(2) {
            let (a, b) = (Vec3::new(w[0][0] as f32, 0.0, w[0][1] as f32), Vec3::new(w[1][0] as f32, 0.0, w[1][1] as f32));
            let ab = b - a;
            if ab.length_squared() < 1e-6 {
                continue;
            }
            let t = ((Vec3::new(p.x, 0.0, p.z) - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let q = a + ab * t;
            let d = q.distance(Vec3::new(p.x, 0.0, p.z));
            if best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, q, ab.normalize()));
            }
        }
    }
    best.map(|(_, q, d)| (q, d))
}

/// A chain's centreline point (plan) and unit tangent and normal at station `s`.
pub(super) fn at(c: &Chain, s: f64) -> (Vec3, Vec3, Vec3) {
    let f = c.frame(s);
    (Vec3::new(f.x as f32, c.y_at(s) as f32, f.z as f32), Vec3::new(f.tx as f32, 0.0, f.tz as f32).normalize_or_zero(), Vec3::new(f.nx as f32, 0.0, f.nz as f32).normalize_or_zero())
}

/// The street a paved crossing passes over (`Crossing::over` is "<name> (<run>)").
pub(super) fn crossed_name(over: &str) -> &str {
    over.rsplit_once(" (").map(|(n, _)| n).unwrap_or(over)
}

/// The built Route 80 deck the drive takes (see the module doc).
fn drive_chain<'a>(world: &World, sw: &'a StructureWorld) -> Option<&'a Chain> {
    let anchor = junction(world, "5th Street", "Harrison Street");
    let i80: Vec<&Chain> = sw.chains().iter().filter(|c| drawn_deck(c) && c.class == RoadClass::Motorway && named(c, "Route 80")).collect();
    let near = anchor.and_then(|a| {
        i80.iter().filter(|c| c.length() >= DRIVE_M).min_by(|x, y| x.project(a.x as f64, a.z as f64).1.total_cmp(&y.project(a.x as f64, a.z as f64).1)).copied()
    });
    let longest = |cs: &mut dyn Iterator<Item = &'a Chain>| cs.max_by(|a, b| a.length().total_cmp(&b.length()).then(b.id.cmp(&a.id)));
    near.or_else(|| longest(&mut i80.iter().copied())).or_else(|| longest(&mut sw.chains().iter().filter(|c| drawn_deck(c) && c.class == RoadClass::Motorway)))
}

pub fn freeway_drive_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    let sw = world.structures.as_ref().filter(|s| !s.is_empty())?;
    let c = drive_chain(world, sw)?;
    let len = c.length();
    let run = DRIVE_M.min(len - 2.0).max(1.0);
    // start where the drive ends as near the middle of the chain as it can.
    let s0 = ((len - run) / 2.0).max(0.0);
    let n_samples = 257;
    let samples: Vec<(Vec3, Vec3)> = (0..n_samples)
        .map(|k| {
            let s = s0 + run * k as f64 / (n_samples - 1) as f64;
            let (p, t, n) = at(c, s);
            // the right lane of travel along increasing station: half of the half width to the right (-n).
            let lane = -(c.half_at(s) as f32) * 0.5;
            (Vec3::new(p.x, p.y + DRIVE_EYE_M, p.z) + n * lane, t)
        })
        .collect();
    let (p0, p1) = (samples[0].0, samples[n_samples - 1].0);
    Some(Segment {
        name: "freeway_drive",
        description: format!(
            "eye {DRIVE_EYE_M} m over the deck of {} (chain {}, {:.0} m built, level {}) in its right lane, {run:.0} m from station {s0:.0} over the path ({:.0}, {:.0}) to ({:.0}, {:.0}), looking ahead with a +-{DRIVE_SWEEP_DEG} degree sweep",
            c.name(),
            c.id,
            len,
            c.level,
            p0.x,
            p0.z,
            p1.x,
            p1.z
        ),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let k = ((t * (n_samples - 1) as f32).round() as usize).min(n_samples - 1);
            let (eye, d) = samples[k];
            let yaw = yaw_of(d) + DRIVE_SWEEP_DEG.to_radians() * (std::f32::consts::TAU * t).sin();
            Camera { far, ..Camera::new(eye, yaw, -0.03, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

/// A point on a street under a built deck: (eye on the street surface + 1.7 m,
/// the street's direction, the chain, what it is).
pub(super) fn under_deck(world: &World, sw: &StructureWorld) -> Option<(Vec3, Vec3, u32, String)> {
    let decks: Vec<&Chain> = sw.chains().iter().filter(|c| drawn_deck(c) && matches!(c.class, RoadClass::Motorway | RoadClass::MotorwayLink)).collect();
    let crossing_of = |street: &str| -> Option<(&Chain, f64)> {
        decks.iter().find_map(|c| c.crossings.iter().find(|x| x.chain.is_none() && crossed_name(&x.over) == street).map(|x| (*c, (x.s0 + x.s1) / 2.0)))
    };
    // 5th Street under the I-80 approach, else any street a built freeway deck crosses.
    let found = crossing_of("5th Street").map(|(c, s)| (c, s, "5th Street".to_string())).or_else(|| {
        decks.iter().find_map(|c| c.crossings.iter().find(|x| x.chain.is_none()).map(|x| (*c, (x.s0 + x.s1) / 2.0, crossed_name(&x.over).to_string())))
    });
    let (c, s, street) = found?;
    let (p, t, _) = at(c, s);
    let (q, dir) = street_point(world, &street, p).unwrap_or((Vec3::new(p.x, 0.0, p.z), Vec3::new(-t.z, 0.0, t.x)));
    let eye = Vec3::new(q.x, surface(world, q.x, q.z) + EYE_HEIGHT_M, q.z);
    Some((eye, dir, c.id, format!("{street} under {} (chain {}, deck top {:.1} m, {:.1} m over the eye)", c.name(), c.id, p.y, p.y - eye.y)))
}

pub fn under_viaduct_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    let sw = world.structures.as_ref().filter(|s| !s.is_empty())?;
    let (eye, dir, _, what) = under_deck(world, sw)?;
    Some(Segment {
        name: "under_viaduct",
        description: format!("eye {EYE_HEIGHT_M} m at ({:.0}, {:.0}) on {what}, looking along the street, the view sweeping +-{UNDER_SWEEP_DEG} degrees across the deck overhead once each way", eye.x, eye.z),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let yaw = yaw_of(dir) + UNDER_SWEEP_DEG.to_radians() * (std::f32::consts::TAU * t).sin();
            Camera { far, ..Camera::new(eye, yaw, 0.12, aspect) }
        }),
        frames,
        paced: false,
        streams: false,
    })
}

/// The structure world the cards are placed on: the drawn one, else one
/// built here for the poses alone (so a run without `--structures` takes the
/// very same cameras).
pub(super) fn poses_world(world: &World) -> Option<Arc<StructureWorld>> {
    if let Some(s) = &world.structures {
        return Some(s.clone());
    }
    let streets = world.street.as_ref()?;
    let standards = eo_data::standards::CaStandards::load(&world.paths).ok()?;
    let t = std::time::Instant::now();
    let w = StructureWorld::build(&streets.world, &world.city, &standards, eo_landmarks::LandmarksJs::get(), Default::default()).ok()?;
    eprintln!("structures-shots: structures built for the poses only ({:.0} ms; not drawn)", t.elapsed().as_secs_f64() * 1e3);
    Some(Arc::new(w))
}

/// The near packs' streaming, for the bench JSON (as the street's).
pub fn stats_json(s: &eo_render::structures::StructuresRuntime) -> serde_json::Value {
    let st = &s.stats;
    let a = s.arena_use();
    serde_json::json!({
        "resident_packs": st.resident, "resident_triangles": st.resident_triangles,
        "resident_vertex_bytes": st.resident_vertex_bytes, "resident_index_bytes": st.resident_index_bytes,
        "packs_cooked": st.packs_cooked, "packs_uploaded": st.packs_uploaded, "packs_evicted": st.packs_evicted,
        "packs_dropped_unneeded": st.packs_dropped_unneeded, "packs_skipped_arena_full": st.packs_skipped_arena_full,
        "worker_ms_sum": st.worker_ms, "max_cook_ms": st.max_cook_ms,
        "arena_vertices_used_peak": [a.0, a.1], "arena_indices_used_peak": [a.2, a.3], "arena_objects_used_peak": [a.4, a.5],
        "arena_capacity": [s.arena.vertex_capacity, s.arena.index_capacity, s.arena.object_capacity],
    })
}
