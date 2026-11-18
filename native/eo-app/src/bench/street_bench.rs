//! Slice 4 bench pieces: the `sidewalk_walk` path (on a footway of Castro
//! Street, across a junction on its crosswalk, on along the next block) and the
//! `capture-s4-*` cameras. Every eye stands on the drawn street surface.

use super::{EYE_HEIGHT_M, Segment, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use eo_render::street::StreetRuntime;
use eo_street::StreetWorld;
use glam::Vec3;
use serde_json::json;

/// Metres walked over the measured path.
const SIDEWALK_WALK_M: f32 = 140.0;

/// The Castro Street junction the path crosses: the run nearest the kit
/// street's start whose end is a node with painted crossings.
pub struct Junction {
    pub node: usize,
    /// Unit direction along the run into the node, and the run's +n.
    pub t: Vec3,
    pub n: Vec3,
    pub node_pos: Vec3,
    /// Lateral offset of the footway's middle, and the footway side (+1/-1).
    pub walk_u: f32,
    pub side: f32,
    pub entry: usize,
}

pub fn junction(sw: &StreetWorld) -> Option<Junction> {
    let target = super::kit_street()[0]; // the same junction on the scaled plan
    let mut best: Option<(f32, usize, usize)> = None;
    for (ei, e) in sw.plan.entries.iter().enumerate() {
        if sw.city.segments[e.road].street_name != "Castro Street" {
            continue;
        }
        let Some((ni, _)) = e.approach_end else { continue };
        let node = &sw.plan.nodes[ni];
        if !node.earns_paint || node.approaches.len() < 4 {
            continue;
        }
        let p = node.position;
        let d = ((p[0] as f32 - target[0]).powi(2) + (p[1] as f32 - target[1]).powi(2)).sqrt();
        if best.map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, ei, ni));
        }
    }
    let (_, ei, ni) = best?;
    let e = &sw.plan.entries[ei];
    let f = e.frame(e.length, false);
    let t = Vec3::new(f.tx as f32, 0.0, f.tz as f32);
    let n = Vec3::new(f.nx as f32, 0.0, f.nz as f32);
    let (side, walk) = if e.walks[0] >= e.walks[1] { (1.0, e.walks[0]) } else { (-1.0, e.walks[1]) };
    let p = sw.plan.nodes[ni].position;
    Some(Junction { node: ni, t, n, node_pos: Vec3::new(p[0] as f32, 0.0, p[1] as f32), walk_u: side * (e.half + walk * 0.5) as f32, side, entry: ei })
}

fn on_surface(sw: &StreetWorld, p: Vec3, up: f32) -> Vec3 {
    Vec3::new(p.x, sw.surface_at(p.x as f64, p.z as f64, None) as f32 + up, p.z)
}

pub fn sidewalk_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    let sw = world.street.as_ref()?.world.clone();
    let j = junction(&sw)?;
    let walk_m = SIDEWALK_WALK_M * eo_data::groundscale::scale() as f32; // the same stretch at a scaled plan
    let start = j.node_pos - j.t * (walk_m * 0.5) + j.n * j.walk_u;
    let eyes: Vec<Vec3> = (0..=1024).map(|i| on_surface(&sw, start + j.t * (walk_m * i as f32 / 1024.0), EYE_HEIGHT_M)).collect();
    let yaw0 = yaw_of(j.t);
    let node = &sw.plan.nodes[j.node];
    Some(Segment {
        name: "sidewalk_walk",
        description: format!(
            "eye {EYE_HEIGHT_M} m above the drawn footway of Castro Street ({:.1} m off the centreline), {SIDEWALK_WALK_M} m over the segment through the junction at ({:.0}, {:.0}) ({} legs, crossings painted) on its crosswalk and on along the next block, the view swinging +-35 degrees; street tiles stream during the measured frames",
            j.walk_u, node.position[0], node.position[1], node.approaches.len()
        ),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let eye = eyes[((t * 1024.0) as usize).min(1024)];
            let yaw = yaw0 + 35f32.to_radians() * (std::f32::consts::TAU * 1.5 * t).sin();
            Camera { far, ..Camera::new(eye, yaw, -0.03, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

pub fn stats_json(s: &StreetRuntime) -> serde_json::Value {
    let st = &s.stats;
    let a = s.arena_use();
    json!({
        "resident_packs": st.resident, "resident_triangles": st.resident_triangles,
        "resident_vertex_bytes": st.resident_vertex_bytes, "resident_index_bytes": st.resident_index_bytes,
        "packs_cooked": st.packs_cooked, "packs_uploaded": st.packs_uploaded, "packs_evicted": st.packs_evicted,
        "packs_dropped_unneeded": st.packs_dropped_unneeded, "packs_skipped_arena_full": st.packs_skipped_arena_full,
        "worker_ms_sum": st.worker_ms, "max_cook_ms": st.max_cook_ms,
        "arena_vertices_used_peak": [a.0, a.1], "arena_indices_used_peak": [a.2, a.3], "arena_objects_used_peak": [a.4, a.5],
    })
}

fn look(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    let pitch = (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan();
    Camera { far, ..Camera::new(eye, yaw_of(to), pitch, aspect) }
}

/// The steepest street run with footways both sides (grade over its drawn run).
fn steepest(sw: &StreetWorld) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (ei, g) in sw.geoms.iter().enumerate() {
        let e = &sw.plan.entries[ei];
        if g.stations.len() < 2 || e.walks[0] < 2.0 || e.walks[1] < 2.0 || e.class_rank != 3 {
            continue;
        }
        let run = (g.stations[g.stations.len() - 1].s - g.stations[0].s) as f32;
        if run < 60.0 {
            continue;
        }
        let grade = ((g.datums[g.datums.len() - 1] - g.datums[0]) as f32 / run).abs();
        // Under 35 %: the steepest real blocks, not a mapping error.
        if grade < 0.35 && best.map(|b| grade > b.1).unwrap_or(true) {
            best = Some((ei, grade));
        }
    }
    best
}

/// The `capture-s4-*` cameras: (name, camera, what it shows).
pub fn s4_cameras(world: &World, aspect: f32, far: f32) -> Vec<(String, Camera, String)> {
    let Some(set) = world.street.as_ref() else { return vec![] };
    let sw = set.world.clone();
    let mut out = vec![];
    let Some(j) = junction(&sw) else { return out };
    let node = &sw.plan.nodes[j.node];
    let e = &sw.plan.entries[j.entry];
    let app = node.approaches.iter().find(|a| a.entry == j.entry).cloned();
    // (a) on the footway at eye height, 35 m before the junction, looking along the street.
    let p = j.node_pos - j.t * 35.0 + j.n * j.walk_u;
    let eye = on_surface(&sw, p, EYE_HEIGHT_M);
    let at = on_surface(&sw, j.node_pos + j.t * 40.0 + j.n * (j.walk_u * 0.4), 1.2);
    out.push(("sidewalk".into(), look(eye, at, aspect, far), format!("footway of Castro Street at eye height, 35 m before the junction at ({:.0}, {:.0})", node.position[0], node.position[1])));
    // (b) the junction: from the footway beyond the crossing, back at the corner, crosswalk and limit line.
    if let Some(app) = &app {
        let d = (app.trim + 9.0) as f32;
        let p = j.node_pos - j.t * d + j.n * (j.walk_u * 0.9);
        let eye = on_surface(&sw, p, 3.2);
        let corner = j.node_pos - j.t * (app.trim as f32 * 0.6) + j.n * (j.side * e.half as f32);
        let at = on_surface(&sw, corner, 0.0);
        out.push(("intersection".into(), look(eye, at, aspect, far), "junction from the footway 3.2 m up: crosswalk bars, limit line, kerb return and kerb ramps".into()));
    }
    // (c) the steepest block: from its lower end's footway, uphill along it.
    if let Some((ei, grade)) = steepest(&sw) {
        let g = &sw.geoms[ei];
        let e = &sw.plan.entries[ei];
        let (lo, hi) = if g.datums[0] < g.datums[g.datums.len() - 1] { (0, g.stations.len() - 1) } else { (g.stations.len() - 1, 0) };
        let (a, b) = (g.stations[lo], g.stations[hi]);
        let dir = Vec3::new((b.x - a.x) as f32, 0.0, (b.z - a.z) as f32).normalize();
        let n = Vec3::new(a.nx as f32, 0.0, a.nz as f32);
        let side = if e.walks[0] >= e.walks[1] { 1.0 } else { -1.0 };
        let u = side * (e.half + (if side > 0.0 { e.walks[0] } else { e.walks[1] }) * 0.4) as f32;
        let p = Vec3::new(a.x as f32, 0.0, a.z as f32) + dir * 8.0 + n * u;
        let eye = on_surface(&sw, p, EYE_HEIGHT_M);
        let at = on_surface(&sw, p + dir * 30.0 + n * (u * 0.3), EYE_HEIGHT_M + 1.0);
        out.push(("hill".into(), look(eye, at, aspect, far), format!("{} ({:.0} % average grade), uphill from the footway", sw.city.segments[e.road].street_name, grade * 100.0)));
    }
    // (d) a kit building's ground floor at the footway: the first facade met behind the walk.
    let g = &sw.geoms[j.entry];
    let walk = if j.side > 0.0 { e.walks[0] } else { e.walks[1] };
    'found: for k in (0..g.stations.len()).rev() {
        let f = g.stations[k];
        for step in 1..60 {
            let u = j.side as f64 * (e.half + walk + step as f64 * 0.15);
            let p = f.at(u);
            if sw.buildings.contains(p[0], p[1]) {
                let wall = Vec3::new(p[0] as f32, 0.0, p[1] as f32);
                let out_dir = -j.n * j.side;
                let eye = on_surface(&sw, wall + out_dir * 5.5 + j.t * 2.0, EYE_HEIGHT_M);
                let at = on_surface(&sw, wall - j.t * 1.0, 1.4);
                out.push(("kit-ground-floor".into(), look(eye, at, aspect, far), format!("ground floor of the building at ({:.0}, {:.0}) behind the Castro Street footway, from 5.5 m out", p[0], p[1])));
                break 'found;
            }
        }
    }
    // (surfaces) close views of the street and facade materials, and of the
    // building-corner bank wedge the `surface_defects` probe found.
    {
        // Asphalt, gutter, kerb and paving: 1.4 m over the carriageway 1.6 m
        // inside the kerb, 22 m before the junction, looking down at the kerb 5 m on.
        let p = j.node_pos - j.t * 22.0 + j.n * (j.side * (e.half as f32 - 1.6));
        let eye = on_surface(&sw, p, 1.4);
        let at = on_surface(&sw, j.node_pos - j.t * 17.0 + j.n * (j.side * (e.half as f32 + 0.8)), 0.0);
        out.push(("surf-asphalt".into(), look(eye, at, aspect, far), "close view: asphalt, gutter pan, kerb and footway paving, 1.4 m over the carriageway".into()));
    }
    'win: for k in (0..g.stations.len()).rev() {
        let f = g.stations[k];
        for step in 1..60 {
            let u = j.side as f64 * (e.half + walk + step as f64 * 0.15);
            let p = f.at(u);
            if sw.buildings.contains(p[0], p[1]) {
                let wall = Vec3::new(p[0] as f32, 0.0, p[1] as f32);
                let out_dir = -j.n * j.side;
                let eye = on_surface(&sw, wall + out_dir * 6.0 - j.t * 1.5, EYE_HEIGHT_M);
                let floor = sw.surface_at(p[0], p[1], None) as f32;
                let at = Vec3::new(wall.x + j.t.x * 1.0, floor + 4.6, wall.z + j.t.z * 1.0);
                out.push(("surf-window".into(), look(eye, at, aspect, far), "close view: second-storey windows of the kit building behind the Castro Street footway, 6 m out".into()));
                break 'win;
            }
        }
    }
    {
        // A verge bank over the frontage paving beside a building corner
        // (eo-street/examples/surface_defects.rs, tile around the Castro junction).
        let wedge = super::on_plan(Vec3::new(-1324.5, 0.0, 2344.5));
        let mut eye = None;
        for q in 0..16 {
            let a = q as f32 * std::f32::consts::TAU / 16.0;
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            let clear = (1..=8).all(|m| !sw.buildings.contains((wedge.x + d.x * m as f32) as f64, (wedge.z + d.z * m as f32) as f64));
            if clear {
                eye = Some(on_surface(&sw, wedge + d * 7.0, EYE_HEIGHT_M));
                break;
            }
        }
        if let Some(eye) = eye {
            let at = on_surface(&sw, wedge, 0.3);
            out.push(("surf-wedge".into(), look(eye, at, aspect, far), "building corner at (-1324.5, 2344.5) where the verge bank stood over the frontage paving".into()));
        }
    }
    // (e) from above the junction.
    let eye = j.node_pos + Vec3::new(0.0, sw.ground(j.node_pos.x as f64, j.node_pos.z as f64) as f32 + 55.0, 0.0) - j.t * 45.0 + j.n * 20.0;
    let at = on_surface(&sw, j.node_pos, 0.0);
    out.push(("above".into(), look(eye, at, aspect, far), "the junction from 55 m up".into()));
    // Every camera must stand above what is drawn under it.
    out.retain(|(name, cam, _)| {
        let s = sw.surface_at(cam.eye.x as f64, cam.eye.z as f64, None) as f32;
        let ok = cam.eye.y > s + 0.5;
        if !ok {
            eprintln!("capture-s4-{name}: eye {:.2} not above the surface {s:.2}; skipped", cam.eye.y);
        }
        ok
    });
    out
}
