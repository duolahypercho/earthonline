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

/// A shallow dive off the Ferry Building. Check the local shore and bed so
/// the capture and the interactive fly camera start inside navigable water.
pub(crate) fn dive_camera(world: &World, aspect: f32, far: f32) -> Option<Camera> {
    let eye = super::on_plan(Vec3::new(FERRY_BUILDING_SURVEYED[0] + 550.0, sea() - 0.4, FERRY_BUILDING_SURVEYED[1] - 250.0));
    let field = &world.water.as_ref()?.field;
    let bed = field.press_ground(eye.x as f64, eye.z as f64, world.elevation.ground(eye.x as f64, eye.z as f64));
    if signed(world, eye.x, eye.z)? < 50.0 || eye.y - (bed as f32) < 0.6 {
        return None;
    }
    Some(Camera { far, ..Camera::new(eye, yaw_of(Vec3::NEG_X), 0.78, aspect) })
}

pub fn dive_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<Segment> {
    let camera = dive_camera(world, aspect, far)?;
    Some(Segment {
        name: "bay_dive",
        description: "Descend through the wave surface off the Ferry Building, look up through the water, then surface.".into(),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.saturating_sub(1).max(1) as f32;
            let dive = (std::f32::consts::PI * t).sin().powi(2);
            let y = sea() + 1.2 - 2.0 * dive;
            Camera { eye: Vec3::new(camera.eye.x, y, camera.eye.z), pitch: -0.15 + dive * 1.05, ..camera }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

/// Short, level swim through the actual starting views. Explicit segment
/// selection also exposes the paths with the habitats off for matched timing.
pub fn habitat_segments(world: &World, args: &crate::Args, frames: usize, aspect: f32, far: f32) -> Vec<Segment> {
    let mut segments: Vec<Segment> = world.dive_sites.iter().filter_map(|site| {
        let name = match site.habitat {
            eo_render::seabed::Habitat::Eelgrass => "bay_grass",
            eo_render::seabed::Habitat::Sand => "pacific_sand",
            eo_render::seabed::Habitat::Rocky => "black_point_rocks",
        };
        if !args.underwater_habitats && !args.segments.as_ref().is_some_and(|paths| paths.iter().any(|p| p == name)) {
            return None;
        }
        let cam = site.camera(aspect, far);
        Some(Segment {
            name, description: format!("A submerged swim across the {name} site."), frames, paced: false, streams: true,
            camera: Box::new(move |i,n| {
                let t = i as f32 / n.saturating_sub(1).max(1) as f32;
                Camera { eye: cam.eye + cam.right() * (t - 0.5) * 1.2, yaw: cam.yaw + (t - 0.5)*0.32, ..cam }
            }),
        })
    }).collect();
    if (args.cove_detail || args.segments.as_ref().is_some_and(|p| p.iter().any(|n| n == "cove_descent")))
        && let Some(&site) = world.dive_sites.iter().find(|s| s.beach.is_some()) {
        segments.push(Segment {
            name: "cove_descent", description: "Descend offshore from the Aquatic Park beach toward the surveyed basin.".into(),
            frames, paced: false, streams: true,
            camera: Box::new(move |i, n| {
                let t = i as f32 / n.saturating_sub(1).max(1) as f32;
                let mut camera = site.cove_camera(aspect, far, 112.0 * t);
                // Enter through the moving surface before following the descending bed.
                camera.eye.y += (1.0 - t * 8.0).max(0.0) * 1.1;
                camera
            }),
        });
    }
    segments
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
    if let Some(cam) = dive_camera(world, aspect, far) {
        out.push(("bay-submerged", cam, "0.4 m under the bay surface, looking up toward the city".into()));
        let crossing = Camera { eye: Vec3::new(cam.eye.x, sea(), cam.eye.z), pitch: 0.0, ..cam };
        out.push(("bay-waterline", crossing, "at mean sea level facing the city".into()));
    }
    // Beside the cove-facing pile row at the northwest curve. The submerged
    // camera sits above the bed and looks through the gap beside the support.
    if world.dive_sites.iter().any(|s| s.cove_detail && s.beach.is_some()) {
        let centre = super::on_plan(Vec3::new(-626.461, 0.0, -3535.113));
        let eye = centre + Vec3::new(-1.5, sea() - 1.61, -0.55);
        let dir = Vec3::new(1.5, -0.04, 0.55).normalize();
        let cam = Camera { near: 0.045, far, fov_y_deg: 60.0, ..Camera::new(eye, yaw_of(dir), dir.y.asin(), aspect) };
        out.push(("municipal-pile", cam, "Municipal Pier support at the submerged bed".into()));
    }
    for site in &world.dive_sites {
        if let Some(shore) = site.beach {
            // Look down on the same patch of the shallow bed on either side
            // of the interface. The two switch settings retain both poses.
            for (name, height) in [("cove-bed-from-air", 0.16), ("cove-bed-from-water", -0.16)] {
                let eye = Vec3::new(shore.x, sea() + height, shore.y - 26.0);
                let cam = Camera { near: 0.025, far, ..Camera::new(eye, -0.42, -1.35, aspect) };
                out.push((name, cam, "Aquatic Park shallow bed seen across the waterline".into()));
            }
            out.push(("cove-surface", site.surface_camera(aspect, far), "Aquatic Park beach toward Municipal Pier and Black Point".into()));
            out.push(("cove-shallows", site.cove_camera(aspect, far, 0.0), "looking down the cove bank just below the surface".into()));
            out.push(("cove-basin", site.cove_camera(aspect, far, 112.0), "112 m farther offshore on the same transect".into()));
        }
        let (name, description) = match site.habitat {
            eo_render::seabed::Habitat::Eelgrass => ("bay-grass", "near the bed in Aquatic Park Cove"),
            eo_render::seabed::Habitat::Rocky => ("black-point-rocks", "the inhabited rock faces at the western shore of Aquatic Park Cove"),
            eo_render::seabed::Habitat::Sand => ("pacific-ripples", "near the bed off Ocean Beach"),
        };
        out.push((name, site.camera(aspect, far), description.into()));
    }
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
            if let Some(w) = &world.water {
                // Search offshore for a shallow, submerged sand receiver.
                for k in 1..100 {
                    let q = x - 2.0 * k as f32;
                    let bed = w.field.press_ground(q as f64, z as f64, world.elevation.ground(q as f64, z as f64)) as f32;
                    if w.field.signed_at(q as f64, z as f64) > 0.0 && bed < sea() - 1.2 {
                        let eye = Vec3::new(q, bed + 0.65, z);
                        out.push(("pacific-bed", Camera { far, ..Camera::new(eye, yaw_of(Vec3::NEG_X), -0.36, aspect) },
                            "above the submerged Ocean Beach foreshore, looking down at the sand".into()));
                        break;
                    }
                }
            }
            break;
        }
    }
    out
}
