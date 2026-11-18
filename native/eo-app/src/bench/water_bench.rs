//! the bench paths along the water (`waterfront`: the Embarcadero
//! promenade by the Ferry Building, at street level; `bay`: a pass over the
//! bay, 20 m up, looking back at the city) and the `capture-water-*` cameras.
//! Where the water is comes from the shore field (eo-water), not from typed
//! coordinates alone: every eye is checked against it.

use super::{EYE_HEIGHT_M, Segment, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use glam::Vec3;

/// The Ferry Building's clock tower (the authored building b558731934's anchor).
pub const FERRY_BUILDING_SURVEYED: [f32; 2] = [2281.5, -1936.6];

/// the Ferry Building on the scaled plan.
pub fn ferry_building() -> [f32; 2] {
    eo_data::groundscale::plan32(FERRY_BUILDING_SURVEYED)
}
/// Metres walked along the promenade over the measured path.
const PROMENADE_WALK_M: f32 = 160.0;
/// The bay pass: metres flown, height above the sea, distance off the shore.
const BAY_PASS_M: f32 = 900.0;
const BAY_HEIGHT_M: f32 = 20.0;

fn sea() -> f32 {
    eo_water::SEA_LEVEL_Y as f32
}

/// Signed shore distance at a point (+ water), or None without water data.
fn signed(world: &World, x: f32, z: f32) -> Option<f32> {
    world.water.as_ref().map(|w| w.field.signed_at(x as f64, z as f64) as f32)
}

fn ground(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// The Embarcadero's points nearest the Ferry Building, as a polyline sorted
/// along it (both carriageways' points, ordered on their principal axis), and
/// that axis (pointing south-east, away from the Ferry Building's north).
fn embarcadero(world: &World) -> (Vec<Vec3>, Vec3) {
    let fb = glam::Vec2::from(ferry_building());
    let mut pts: Vec<glam::Vec2> = world
        .city
        .roads
        .iter()
        .filter(|r| r.name == "The Embarcadero")
        .flat_map(|r| r.points.iter().map(|p| glam::Vec2::new(p[0], p[1])))
        .filter(|p| p.distance(fb) < 700.0 * super::gsf())
        .collect();
    if pts.len() < 2 {
        return (vec![], Vec3::X);
    }
    // Order along the principal direction of the points.
    let mean = pts.iter().fold(glam::Vec2::ZERO, |a, p| a + *p) / pts.len() as f32;
    let (mut sxx, mut sxz, mut szz) = (0.0f32, 0.0f32, 0.0f32);
    for p in &pts {
        let d = *p - mean;
        sxx += d.x * d.x;
        sxz += d.x * d.y;
        szz += d.y * d.y;
    }
    let ang = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let mut axis = glam::Vec2::new(ang.cos(), ang.sin());
    if axis.y < 0.0 {
        axis = -axis; // south-east (z grows south)
    }
    pts.sort_by(|a, b| a.dot(axis).partial_cmp(&b.dot(axis)).unwrap());
    pts.dedup_by(|a, b| a.distance(*b) < 1.0);
    // One point per 10 m of the axis (the two carriageways averaged).
    let mut out: Vec<Vec3> = vec![];
    let (lo, hi) = (pts[0].dot(axis), pts[pts.len() - 1].dot(axis));
    let mut s = lo;
    while s <= hi {
        let near: Vec<&glam::Vec2> = pts.iter().filter(|p| (p.dot(axis) - s).abs() <= 25.0).collect();
        if !near.is_empty() {
            let m = near.iter().fold(glam::Vec2::ZERO, |a, p| a + **p) / near.len() as f32;
            out.push(Vec3::new(m.x, 0.0, m.y));
        }
        s += 10.0;
    }
    (out, Vec3::new(axis.x, 0.0, axis.y))
}

/// From a road point, the promenade: walk toward the water until the shore
/// is `inland` metres away; returns that point and the unit direction to the water.
fn promenade_point(world: &World, p: Vec3, along: Vec3, inland: f32) -> Option<(Vec3, Vec3)> {
    let n = Vec3::new(-along.z, 0.0, along.x);
    let s_plus = signed(world, p.x + n.x * 120.0, p.z + n.z * 120.0)?;
    let s_minus = signed(world, p.x - n.x * 120.0, p.z - n.z * 120.0)?;
    let to_water = if s_plus >= s_minus { n } else { -n };
    for k in 0..240 {
        let q = p + to_water * k as f32 * 0.5;
        if signed(world, q.x, q.z)? > -inland {
            return Some((q, to_water));
        }
    }
    None
}

fn polyline_at(path: &[Vec3], s: f32) -> (Vec3, Vec3) {
    let mut left = s.max(0.0);
    for w in path.windows(2) {
        let d = w[1] - w[0];
        let l = d.length();
        if left <= l {
            return (w[0] + d * (left / l.max(1e-6)), d.normalize_or(Vec3::X));
        }
        left -= l;
    }
    let n = path.len();
    (path[n - 1], (path[n - 1] - path[n - 2]).normalize_or(Vec3::X))
}

/// Where the promenade walk is centred: this far south-east of the Ferry
/// Building along The Embarcadero (Rincon Park and Pier 14, where the
/// promenade runs on the seawall with the open bay beside it).
const WALK_CENTRE_SE_M: f32 = 320.0;

/// The road point (arc length) nearest a world point.
fn arc_nearest(road: &[Vec3], p: Vec3) -> f32 {
    let mut best = (f32::MAX, 0.0f32);
    let mut acc = 0.0;
    for w in road.windows(2) {
        let d = w[1] - w[0];
        let l = d.length();
        let t = ((p - w[0]).dot(d) / (l * l).max(1e-6)).clamp(0.0, 1.0);
        let dist = (w[0] + d * t).distance(p);
        if dist < best.0 {
            best = (dist, acc + t * l);
        }
        acc += l;
    }
    best.1
}

/// The promenade walk: `PROMENADE_WALK_M` along The Embarcadero south-east
/// of the Ferry Building, on the bay side 6 m from the shoreline, eye 1.7 m
/// over the drawn surface, looking out over the bay with a +-55 degree sweep.
pub fn waterfront_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    let (road, axis) = embarcadero(world);
    if road.len() < 2 {
        eprintln!("water: The Embarcadero not found; the waterfront path left out");
        return None;
    }
    let total: f32 = road.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let fb = Vec3::new(ferry_building()[0], 0.0, ferry_building()[1]);
    let centre = arc_nearest(&road, fb + axis * WALK_CENTRE_SE_M * super::gsf());
    let s0 = (centre - PROMENADE_WALK_M * 0.5).clamp(0.0, (total - PROMENADE_WALK_M).max(0.0));
    let mut eyes = Vec::with_capacity(1025);
    let mut looks = Vec::with_capacity(1025);
    for i in 0..=1024 {
        let (p, _) = polyline_at(&road, s0 + PROMENADE_WALK_M * i as f32 / 1024.0);
        let Some((q, to_water)) = promenade_point(world, p, axis, 6.0) else {
            eprintln!("water: no shoreline beside The Embarcadero at ({:.0}, {:.0}); the waterfront path left out", p.x, p.z);
            return None;
        };
        eyes.push(Vec3::new(q.x, ground(world, q.x, q.z) + EYE_HEIGHT_M, q.z));
        looks.push(to_water);
    }
    // Smooth the look direction (the shoreline is jagged at the piers).
    let smooth: Vec<Vec3> = (0..looks.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(64), (i + 64).min(looks.len() - 1));
            looks[a..=b].iter().fold(Vec3::ZERO, |s, v| s + *v).normalize_or(Vec3::X)
        })
        .collect();
    let (start, end) = (eyes[0], eyes[1024]);
    Some(Segment {
        name: "waterfront",
        description: format!(
            "eye {EYE_HEIGHT_M} m above the drawn surface of the Embarcadero promenade, 6 m inland of the shoreline (shore field), {PROMENADE_WALK_M} m along The Embarcadero centred {WALK_CENTRE_SE_M} m south-east of the Ferry Building ({:.0}, {:.0}) -> ({:.0}, {:.0}), looking out over the bay with a +-55 degree sweep, one and a half times over the segment",
            start.x, start.z, end.x, end.z
        ),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let k = ((t * 1024.0) as usize).min(1024);
            let yaw = yaw_of(smooth[k]) + 55f32.to_radians() * (std::f32::consts::TAU * 1.5 * t).sin();
            Camera { far, ..Camera::new(eyes[k], yaw, -0.04, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

/// The bay pass: 20 m over the water, a line 1.1 km off the Ferry Building
/// heading north for 900 m, looking back west at the city (+-30 degrees).
pub fn bay_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    // the same line over the bay on the scaled plan (offsets scaled; the pass flies BAY_PASS_M).
    let g = super::gsf();
    let x = (FERRY_BUILDING_SURVEYED[0] + 1100.0) * g;
    let (z0, z1) = ((FERRY_BUILDING_SURVEYED[1] + 450.0) * g, (FERRY_BUILDING_SURVEYED[1] + 450.0) * g - BAY_PASS_M);
    // Every eye over open water (checked, not assumed).
    for k in 0..=8 {
        let z = z0 + (z1 - z0) * k as f32 / 8.0;
        if signed(world, x, z)? < 50.0 {
            eprintln!("water: the bay pass at ({x:.0}, {z:.0}) is within 50 m of land; path left out");
            return None;
        }
    }
    let y = sea() + BAY_HEIGHT_M;
    Some(Segment {
        name: "bay",
        description: format!("eye {BAY_HEIGHT_M} m above the sea at x = {x:.0}, 1.1 km off the Ferry Building, flying north from z = {z0:.0} to {z1:.0} ({BAY_PASS_M} m) looking back west at the city with a +-30 degree sweep (the skyline and its reflection in view)"),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let eye = Vec3::new(x, y, z0 + (z1 - z0) * t);
            let yaw = yaw_of(Vec3::NEG_X) + 30f32.to_radians() * (std::f32::consts::TAU * t).sin();
            Camera { far, ..Camera::new(eye, yaw, -0.07, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

/// The `capture-water-*` views: (name, camera, what it shows).
pub fn capture_cameras(world: &World, aspect: f32, far: f32) -> Vec<(&'static str, Camera, String)> {
    let mut out = vec![];
    let (road, axis) = embarcadero(world);
    let fb = Vec3::new(ferry_building()[0], 0.0, ferry_building()[1]);
    let g = super::gsf();
    if road.len() >= 2 {
        // The promenade south-east of the Ferry Building, looking out across the bay
        // toward the Bay Bridge's line (the walk's centre).
        let (p, _) = polyline_at(&road, arc_nearest(&road, fb + axis * WALK_CENTRE_SE_M * g));
        if let Some((q, to_water)) = promenade_point(world, p, axis, 6.0) {
            let eye = Vec3::new(q.x, ground(world, q.x, q.z) + EYE_HEIGHT_M, q.z);
            let dir = (to_water * 0.85 + axis * 0.5).normalize();
            out.push(("embarcadero", Camera { far, ..Camera::new(eye, yaw_of(dir), -0.03, aspect) }, "the Embarcadero promenade south-east of the Ferry Building, eye 1.7 m, looking out over the bay".to_string()));
        }
        // A pier: from the water 3 m up, off the piers north of the Ferry Building, looking back at them.
        let (p, _) = polyline_at(&road, arc_nearest(&road, fb - axis * 380.0 * g));
        if let Some((q, to_water)) = promenade_point(world, p, axis, 6.0) {
            let off = q + (to_water * 260.0 + axis * 120.0) * g;
            let eye_p = Vec3::new(off.x, sea() + 3.0, off.z);
            let dir_p = (-to_water * 0.8 - axis * 0.6).normalize();
            out.push(("pier", Camera { far, ..Camera::new(eye_p, yaw_of(dir_p), -0.02, aspect) }, "the piers north of the Ferry Building from the water, 3 m up".to_string()));
        }
    }
    // The skyline across the bay: 1 km off the Ferry Building, 8 m up, looking west.
    let eye = super::on_plan(Vec3::new(FERRY_BUILDING_SURVEYED[0] + 1000.0, sea() + 8.0, FERRY_BUILDING_SURVEYED[1] + 150.0));
    out.push(("skyline-reflection", Camera { far, ..Camera::new(eye, yaw_of(Vec3::new(-1.0, 0.0, 0.12)), -0.035, aspect) }, "downtown across the bay from 1 km out, 8 m over the water".to_string()));
    // The bay from above: 1.6 km up over the water east of the city, looking down-west.
    let eye = super::on_plan(Vec3::new(FERRY_BUILDING_SURVEYED[0] + 2600.0, 1600.0, FERRY_BUILDING_SURVEYED[1] + 900.0));
    let to = super::on_plan(Vec3::new(FERRY_BUILDING_SURVEYED[0] - 600.0, 0.0, FERRY_BUILDING_SURVEYED[1])) - eye;
    out.push(("bay-aerial", Camera { far: far.max(9000.0), ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect) }, "the bay and the city front from 1.6 km up".to_string()));
    // Ocean Beach: the open Pacific's shore, on the sand looking west.
    for x in (0..60).map(|k| (-7600.0 - k as f32 * 10.0) * g) {
        let z = 1500.0 * g;
        if signed(world, x, z).is_some_and(|s| s > -40.0) {
            let eye = Vec3::new(x, ground(world, x, z).max(sea()) + EYE_HEIGHT_M, z);
            out.push(("ocean-beach", Camera { far, ..Camera::new(eye, yaw_of(Vec3::new(-1.0, 0.0, -0.25)), -0.04, aspect) }, "Ocean Beach, on the sand 40 m from the waterline, looking out at the Pacific".to_string()));
            break;
        }
    }
    out
}
