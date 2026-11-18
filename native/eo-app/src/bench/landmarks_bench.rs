//! the bench paths that see the landmarks (`bay_bridge`: the
//! Embarcadero promenade north-west of the Bay Bridge's San Francisco
//! landing, looking at the west span; `golden_gate`: the Crissy Field shore,
//! looking at the Golden Gate) and the `capture-landmarks-*` cameras. The
//! bridges' positions come from eo-landmarks' export (the JS site data), the
//! shore from the water's shore field: every eye is checked against it.

use super::{EYE_HEIGHT_M, Segment, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use glam::Vec3;

/// Metres walked over a measured path.
const WALK_M: f32 = 160.0;

fn sea() -> f32 {
    eo_water::SEA_LEVEL_Y as f32
}

/// Signed shore distance (+ water), or None without water data.
fn signed(world: &World, x: f32, z: f32) -> Option<f32> {
    world.water.as_ref().map(|w| w.field.signed_at(x as f64, z as f64) as f32)
}

fn ground(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// From `p`, the point `inland` metres from the shore: stepped toward the
/// water (`to_water`) until the shore is that close, or back from it.
fn shore_point(world: &World, p: Vec3, to_water: Vec3, inland: f32) -> Option<Vec3> {
    let mut q = p;
    for _ in 0..600 {
        let s = signed(world, q.x, q.z)?;
        if (s + inland).abs() < 0.6 {
            return Some(q);
        }
        q += to_water * if s < -inland { 0.5 } else { -0.5 };
    }
    None
}

fn v2(p: [f64; 2]) -> Vec3 {
    Vec3::new(p[0] as f32, 0.0, p[1] as f32)
}

/// The Bay Bridge's San Francisco landing (the plan's deck end) and its W3 tower.
fn bay_bridge() -> (Vec3, Vec3) {
    let b = &eo_landmarks::LandmarksJs::get().bay_bridge_west;
    let sf = &b.deck_ends["sf"];
    (Vec3::new(sf.x as f32, 0.0, sf.z as f32), v2([b.towers[1].centre[0], b.towers[1].centre[1]]))
}

/// The Golden Gate's south tower and the middle of its main span.
fn golden_gate() -> (Vec3, Vec3) {
    let g = &eo_landmarks::LandmarksJs::get().golden_gate;
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let s = mid(g.tower_legs["south"].east, g.tower_legs["south"].west);
    let n = mid(g.tower_legs["north"].east, g.tower_legs["north"].west);
    (v2(s), v2(mid(s, n)))
}

/// A walk of `WALK_M` along the shore, `inland` metres from the water,
/// starting `from` along `dir`, the water on the `to_water` side.
fn shore_walk(world: &World, from: Vec3, dir: Vec3, to_water: Vec3, inland: f32) -> Option<Vec<Vec3>> {
    let mut eyes = Vec::with_capacity(1025);
    for i in 0..=1024 {
        let p = from + dir * WALK_M * i as f32 / 1024.0;
        let q = shore_point(world, p, to_water, inland)?;
        eyes.push(Vec3::new(q.x, ground(world, q.x, q.z).max(sea()) + EYE_HEIGHT_M, q.z));
    }
    Some(eyes)
}

/// The Embarcadero walk toward the Bay Bridge: from 330 m to 170 m north-west
/// of the landing along the line to the Ferry Building, 8 m inland of the shore.
fn bay_bridge_walk(world: &World) -> Option<(Vec<Vec3>, Vec3)> {
    let (landing, w3) = bay_bridge();
    let fb = super::water_bench::ferry_building();
    let fb = Vec3::new(fb[0], 0.0, fb[1]);
    let nw = (fb - landing).normalize();
    let to_water = Vec3::new(-nw.z, 0.0, nw.x); // across the line, toward the bay (east-north-east)
    let eyes = shore_walk(world, landing + nw * 330.0 * super::gsf(), -nw, to_water, 8.0)?;
    Some((eyes, w3))
}

/// Crissy Field: 160 m along the shore from 1.05 km east of the Golden Gate's
/// south tower, 10 m inland of the water.
fn crissy_walk(world: &World) -> Option<(Vec<Vec3>, Vec3)> {
    let (south, mid) = golden_gate();
    let start = south + Vec3::new(1050.0, 0.0, 900.0) * super::gsf();
    let eyes = shore_walk(world, start, Vec3::X, Vec3::NEG_Z, 10.0)?;
    Some((eyes, mid))
}

fn walk_segment(name: &'static str, description: String, eyes: Vec<Vec3>, target: Vec3, sweep_deg: f32, frames: usize, aspect: f32, far: f32) -> Segment {
    Segment {
        name,
        description,
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let k = ((t * 1024.0) as usize).min(1024);
            let eye = eyes[k];
            let to = Vec3::new(target.x, eye.y + 30.0, target.z) - eye;
            let yaw = yaw_of(to) + sweep_deg.to_radians() * (std::f32::consts::TAU * 1.5 * t).sin();
            Camera { far, ..Camera::new(eye, yaw, 0.02, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    }
}

pub fn segments(world: &World, frames: usize, aspect: f32, far: f32) -> Vec<Segment> {
    let mut out = vec![];
    match bay_bridge_walk(world) {
        Some((eyes, w3)) => {
            let (a, b) = (eyes[0], eyes[1024]);
            out.push(walk_segment(
                "bay_bridge",
                format!("eye {EYE_HEIGHT_M} m above the drawn surface of the Embarcadero promenade, 8 m inland of the shore (shore field), {WALK_M} m from ({:.0}, {:.0}) to ({:.0}, {:.0}) toward the Bay Bridge's San Francisco landing, looking at its W3 tower ({:.0}, {:.0}) with a +-25 degree sweep", a.x, a.z, b.x, b.z, w3.x, w3.z),
                eyes,
                w3,
                25.0,
                frames,
                aspect,
                far,
            ));
        }
        None => eprintln!("landmarks: no shore beside the Bay Bridge landing; the bay_bridge path left out"),
    }
    match crissy_walk(world) {
        Some((eyes, mid)) => {
            let (a, b) = (eyes[0], eyes[1024]);
            out.push(walk_segment(
                "golden_gate",
                format!("eye {EYE_HEIGHT_M} m above the ground of the Crissy Field shore, 10 m inland of the water (shore field), {WALK_M} m from ({:.0}, {:.0}) to ({:.0}, {:.0}), looking at the middle of the Golden Gate's main span ({:.0}, {:.0}) with a +-20 degree sweep", a.x, a.z, b.x, b.z, mid.x, mid.z),
                eyes,
                mid,
                20.0,
                frames,
                aspect,
                far,
            ));
        }
        None => eprintln!("landmarks: no shore at Crissy Field; the golden_gate path left out"),
    }
    match park_walk(world) {
        Some((eyes, at)) => {
            let (a, b) = (eyes[0], eyes[1024]);
            out.push(walk_segment(
                "park",
                format!("eye {EYE_HEIGHT_M} m above the ground across Union Square (sf-lots-parks way 25278818, a park the JS lays as plaza paving), {:.0} m from ({:.0}, {:.0}) to ({:.0}, {:.0}), looking across it with a +-60 degree sweep", WALK_M * 0.6, a.x, a.z, b.x, b.z),
                eyes,
                at,
                60.0,
                frames,
                aspect,
                far,
            ));
        }
        None => eprintln!("landmarks: Union Square not found; the park path left out"),
    }
    out
}

/// A green's area centroid and its south-west-most corner (sf-lots-parks), by the JS carpet's id.
fn green_centre(world: &World, id: &str) -> Option<(Vec3, Vec3)> {
    let (greens, _) = eo_landmarks::plaza::load_areas(&world.paths.repo_root).ok()?;
    let g = greens.iter().find(|g| g.id == id)?;
    let n = g.polygon.len() as f64;
    let (sx, sz) = g.polygon.iter().fold((0.0, 0.0), |s, p| (s.0 + p[0], s.1 + p[1]));
    let corner = g.polygon.iter().max_by(|a, b| (a[1] - a[0]).total_cmp(&(b[1] - b[0])))?;
    Some((Vec3::new((sx / n) as f32, 0.0, (sz / n) as f32), Vec3::new(corner[0] as f32, 0.0, corner[1] as f32)))
}

/// The park path: across Union Square (a park the JS lays as paving), corner to corner.
fn park_walk(world: &World) -> Option<(Vec<Vec3>, Vec3)> {
    let (c, corner) = green_centre(world, "osm-green-w25278818")?;
    let dir = (c - corner).normalize();
    let eyes = (0..=1024)
        .map(|i| {
            let p = corner + dir * (4.0 + (WALK_M * 0.6) * i as f32 / 1024.0);
            Vec3::new(p.x, ground(world, p.x, p.z) + EYE_HEIGHT_M, p.z)
        })
        .collect();
    Some((eyes, c + dir * 200.0))
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect) }
}

/// The `capture-landmarks-*` views: (name, camera, what it shows).
pub fn capture_cameras(world: &World, aspect: f32, far: f32) -> Vec<(&'static str, Camera, String)> {
    let mut out = vec![];
    if let Some((eyes, w3)) = bay_bridge_walk(world) {
        let eye = eyes[512];
        out.push(("bay-bridge", look_at(eye, Vec3::new(w3.x, 45.0, w3.z), aspect, far), "the Bay Bridge west span from the Embarcadero promenade, eye 1.7 m".to_string()));
    }
    if let Some((eyes, mid)) = crissy_walk(world) {
        let eye = eyes[512];
        out.push(("golden-gate", look_at(eye, Vec3::new(mid.x, 70.0, mid.z), aspect, far), "the Golden Gate from the Crissy Field shore, eye 1.7 m".to_string()));
    }
    // The Embarcadero's quay wall: from the water 30 m off the wall half-way
    // between the Ferry Building and the bridge landing, 2.5 m over the sea,
    // looking back along the wall toward the Ferry Building.
    let fb = super::water_bench::ferry_building();
    let fb = Vec3::new(fb[0], 0.0, fb[1]);
    let (landing, _) = bay_bridge();
    let nw = (fb - landing).normalize();
    let to_water = Vec3::new(-nw.z, 0.0, nw.x);
    if let Some(q) = shore_point(world, landing + nw * 380.0 * super::gsf(), to_water, -30.0) {
        let eye = Vec3::new(q.x, sea() + 2.5, q.z);
        let at = q + nw * 120.0 - to_water * 34.0;
        out.push(("embarcadero-seawall", look_at(eye, Vec3::new(at.x, sea() + 1.0, at.z), aspect, far), "the Embarcadero quay wall from the water, 30 m out, 2.5 m up, looking along it toward the Ferry Building".to_string()));
    }
    // The Ferry Plaza pier deck: from the water 140 m off its east side, 3 m up.
    let fp = &eo_landmarks::LandmarksJs::get().shore.ferry_pier;
    let c = fp.outline.iter().fold([0.0f64; 2], |a, p| [a[0] + p[0], a[1] + p[1]]);
    let c = Vec3::new((c[0] / fp.outline.len() as f64) as f32, 0.0, (c[1] / fp.outline.len() as f64) as f32);
    let eye = c + (to_water * 170.0 + nw * 60.0) * super::gsf();
    out.push(("pier", look_at(Vec3::new(eye.x, sea() + 3.0, eye.z), Vec3::new(c.x, sea() + 2.0, c.z), aspect, far), "the Ferry Plaza pier deck (OSM way 661723975) from the water, 3 m up".to_string()));
    // The Ocean Beach seawall: on the beach 28 m seaward of the Esplanade at
    // the stair between Judah and Kirkham, eye 1.7 m, looking at the wall.
    let ob = &eo_landmarks::LandmarksJs::get().promenades["oceanBeach"]["ESPLANADE"];
    if let (Some(ax), Some(az), Some(bx), Some(bz)) = (ob[0]["x"].as_f64(), ob[0]["z"].as_f64(), ob[1]["x"].as_f64(), ob[1]["z"].as_f64()) {
        let (a, b) = (Vec3::new(ax as f32, 0.0, az as f32), Vec3::new(bx as f32, 0.0, bz as f32));
        let d = (b - a).normalize();
        let sea_n = Vec3::new(-d.z, 0.0, d.x);
        let on = a + d * ((b - a).length() * 0.55);
        let eye = on + sea_n * 28.0 - d * 20.0;
        let eye = Vec3::new(eye.x, ground(world, eye.x, eye.z).max(sea()) + EYE_HEIGHT_M, eye.z);
        let at = on + d * 25.0;
        out.push(("ocean-beach-seawall", look_at(eye, Vec3::new(at.x, eye.y - 0.5, at.z), aspect, far), "the Ocean Beach seawall and promenade from the sand, eye 1.7 m".to_string()));
    }
    // A plaza and a park the JS lays as paving: Justin Herman Plaza and Union Square.
    for (name, id, what) in [("plaza", "osm-green-w27146063", "Justin Herman Plaza (paved green) from its south-east corner, eye 1.7 m"), ("park", "osm-green-w25278818", "Union Square (a park the JS paves) from its south-west corner, eye 1.7 m")] {
        if let Some((c, corner)) = green_centre(world, id) {
            let eye = corner + (c - corner).normalize() * 6.0;
            let eye = Vec3::new(eye.x, ground(world, eye.x, eye.z) + EYE_HEIGHT_M, eye.z);
            out.push((name, look_at(eye, Vec3::new(c.x, eye.y - 1.2, c.z), aspect, far), what.to_string()));
        }
    }
    // The bay from above: the city front and the Bay Bridge from 1.6 km up.
    let (landing, w3) = bay_bridge();
    let g = super::gsf(); // offsets on the scaled plan
    let eye = Vec3::new(landing.x + 1500.0 * g, 1600.0, landing.z + 1900.0 * g);
    out.push(("bay-aerial", Camera { far: far.max(9000.0), ..look_at(eye, Vec3::new(w3.x - 700.0 * g, 0.0, w3.z + 200.0 * g), aspect, far) }, "the bay, the Bay Bridge and the city front from 1.6 km up".to_string()));
    out.extend(landmarks2_cameras(world, aspect, far));
    out
}

/// the landmark buildings' registry anchors and heights
/// (landmark-registry.js, the -site.js modules), for these cameras only.
const LANDMARK_BUILDINGS: [(&str, [f32; 2], f32); 5] = [
    ("transamerica", [1461.85, -1898.15], 260.0),
    ("salesforce", [1980.05, -1302.29], 326.1),
    ("coit", [1192.7, -2697.91], 67.1),
    ("sutro", [-2943.4, 2516.49], 297.8),
    ("city-hall", [4.2, -140.5], 93.7),
];

/// the waterfront and the landmark buildings: the Ferry Plaza
/// pier from the Embarcadero, the Embarcadero's quay from its promenade,
/// Fisherman's Wharf, Aquatic Park and Mission Bay from the water, and each
/// landmark building from the south-east (the noon sun on its lit faces).
fn landmarks2_cameras(world: &World, aspect: f32, far: f32) -> Vec<(&'static str, Camera, String)> {
    let mut out = vec![];
    let fb = super::water_bench::ferry_building();
    let fb = Vec3::new(fb[0], 0.0, fb[1]);
    let g = super::gsf();
    // The Ferry Plaza pier from the promenade north of the Ferry Building, eye 1.7 m, looking south-east along it.
    let fp = &eo_landmarks::LandmarksJs::get().shore.ferry_pier;
    let c = fp.outline.iter().fold([0.0f64; 2], |a, p| [a[0] + p[0], a[1] + p[1]]);
    let c = Vec3::new((c[0] / fp.outline.len() as f64) as f32, 0.0, (c[1] / fp.outline.len() as f64) as f32);
    let _ = fb;
    let eye = c + Vec3::new(170.0 * g, sea() + 45.0, -150.0 * g);
    out.push(("pier-deck", look_at(eye, Vec3::new(c.x, sea() + 2.0, c.z), aspect, far), "the Ferry Plaza pier and its neighbours from 45 m over the water north-east of it".to_string()));
    // Fisherman's Wharf: from the water 40 m off the Jefferson Street basin's
    // mouth (stepped from the wharf toward the bay on the shore field), 4 m up.
    let seed = super::on_plan(Vec3::new(330.0, 0.0, -3480.0));
    let q = shore_point(world, seed, Vec3::NEG_Z, -40.0).unwrap_or(super::on_plan(Vec3::new(330.0, 0.0, -3640.0)));
    out.push(("fishermans-wharf", look_at(Vec3::new(q.x + 60.0 * g, sea() + 4.0, q.z), super::on_plan(Vec3::new(200.0, sea() + 3.0, -3500.0)), aspect, far), "Fisherman's Wharf (Pier 45 and the Jefferson Street basin) from the water, 4 m up".to_string()));
    // Aquatic Park and the Municipal Pier from the water inside the cove, 4 m up.
    let eye = super::on_plan(Vec3::new(-350.0, sea() + 4.0, -3480.0));
    out.push(("aquatic-park", look_at(eye, super::on_plan(Vec3::new(-120.0, sea() + 2.0, -3300.0)), aspect, far), "Aquatic Park's beach and the Hyde Street piers from the cove, 4 m up".to_string()));
    // Mission Bay: Pier 48 and the piers south of it from the water 50 m off
    // Pier 48's bay end (stepped east on the shore field), 4 m up.
    let seed = super::on_plan(Vec3::new(2936.0, 0.0, 246.0));
    let q = shore_point(world, seed, Vec3::X, -50.0).unwrap_or(super::on_plan(Vec3::new(3250.0, 0.0, 300.0)));
    out.push(("mission-bay", look_at(Vec3::new(q.x, sea() + 4.0, q.z + 40.0 * g), Vec3::new(seed.x, sea() + 3.0, seed.z + 260.0 * g), aspect, far), "Mission Bay's piers (Pier 48 southward) from the water, 4 m up".to_string()));
    for (name, at, h) in LANDMARK_BUILDINGS {
        let a = super::on_plan(Vec3::new(at[0], 0.0, at[1])); // a landmark building stands at its scaled anchor
        let g = ground(world, a.x, a.z);
        // Downtown's towers stand round the two tallest: their eyes go over the roofs.
        let tall = h > 200.0;
        let dist = if tall { h * 2.4 } else { (h * 1.9).max(180.0) };
        // From the south-east; the Transamerica Pyramid from the east-north-east
        // (555 California Street stands between it and the south-east).
        let dir = if name == "transamerica" { Vec3::new(0.8, 0.0, -0.6) } else { Vec3::new(0.6, 0.0, 0.8) };
        let e = a + dir * dist;
        let eye = Vec3::new(e.x, ground(world, e.x, e.z).max(g) + if tall { h * 0.75 } else { h * 0.3 + 20.0 }, e.z);
        let label: &'static str = match name {
            "transamerica" => "building-transamerica",
            "salesforce" => "building-salesforce",
            "coit" => "building-coit",
            "sutro" => "building-sutro",
            _ => "building-city-hall",
        };
        out.push((label, look_at(eye, Vec3::new(a.x, g + h * 0.45, a.z), aspect, far), format!("{name}, {dist:.0} m out, eye {:.0} m over its ground", eye.y - g)));
    }
    out
}
