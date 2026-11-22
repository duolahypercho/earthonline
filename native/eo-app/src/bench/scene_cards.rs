//! The visual gate's eight scene cards (Docs/VISUAL_QUALITY_GATE.md,
//! "Required blind side-by-side protocol") at the Ferry Building and The
//! Embarcadero hero corridor: `eo-app --scene-cards --out DIR`, at 2560 x
//! 1440 unless a size is given. Cards 1 to 7 are stills taken as the
//! `--shots` stills are (shots.rs: settled, converged, a manifest each);
//! card 7 is three stills (standing, walking, stopped at the kerb); card 8
//! is a numbered frame sequence, rendered every frame at 60 Hz of wall time
//! with streaming and traffic running, along the promenade across a 128 m
//! tile boundary.
//!
//! Every place is found from the data (the street plan's runs and
//! junctions by street name, the shore field, the Ferry Building's
//! authored anchor), never from typed coordinates alone; a card the world
//! cannot honour (no street world, no people assets, no traffic) is still
//! rendered and its manifest says what is missing.
//!
//! `--shots-only a,b` keeps the cards whose name contains a or b;
//! `--card8-seconds S` (30), `--card8-fps N` (10 frames written per second)
//! and `--card8-speed M` (the player's run, m/s) shape the sequence.

use super::shots::{self, Io, Shot, WATER_CLOCK_S};
use super::{EYE_HEIGHT_M, draw, read_color, water_bench, write_png, yaw_of};
use crate::agents::AgentRuntime;
use crate::{Args, World};
use anyhow::{Result, anyhow};
use eo_render::camera::Camera;
use eo_render::sky::WeatherMode;
use eo_sky::Kind;
use eo_street::plan::Entry;
use eo_street::{Layer, StreetWorld};
use glam::{DVec2, DVec3, Vec3};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

/// The gate's scene categories, verbatim.
const CATEGORIES: [&str; 8] = [
    "pedestrian eye-level commercial street, clear day",
    "intersection/crosswalk with traffic and pedestrians",
    "dense building canyon at golden hour",
    "shoreline/waterfront at daylight",
    "wet street during drizzle or immediately after rain",
    "night street with shop, vehicle, and street lighting",
    "third-person character standing, walking, and stopping at a curb",
    "moving traversal clip (30 seconds) crossing a tile boundary",
];

/// The hours the cards are taken at (the same hours the other capture sets use).
const DAY_H: f64 = 12.0;
const GOLDEN_H: f64 = 18.5;
const NIGHT_H: f64 = 21.0;
const DRIZZLE_H: f64 = 16.0;
/// Card 1 and 6: this far along Market Street from its end at The Embarcadero (surveyed metres).
const MARKET_STAND_M: f64 = 150.0;
/// Card 3: this far along California Street from its east end (surveyed metres).
const CALIFORNIA_STAND_M: f64 = 160.0;
/// Card 5: this far back along The Embarcadero from the point nearest the Ferry Building (surveyed metres).
const DRIZZLE_BACK_M: f64 = 25.0;
/// Card 7: the character's distance from the kerb edge standing, walking toward it, stopped at it.
const CHARACTER_BACK_M: [f64; 3] = [3.0, 1.6, 0.35];
/// Card 8: the promenade path starts this far south-east of the Ferry Building (surveyed metres).
const TRAVERSE_FROM_SE_M: f64 = 60.0;
/// Card 8: the look direction is the path's chord over this many metres either side.
const TRAVERSE_LOOK_M: f64 = 8.0;
/// Card 8: the stretch of The Embarcadero the promenade path follows (a straight line fitted to it), metres.
const TRAVERSE_STRETCH_M: f64 = 420.0;
/// Card 8: the path keeps this far past The Embarcadero's carriageway and this far inland of the shore.
const TRAVERSE_KERB_CLEAR_M: f64 = 4.0;
/// A footway with carriageway this near beyond it is a median.
const MEDIAN_PROBE_M: f64 = 12.0;

fn gs() -> f64 {
    eo_data::groundscale::scale()
}

fn ferry() -> DVec2 {
    let f = water_bench::ferry_building();
    DVec2::new(f[0] as f64, f[1] as f64)
}

fn street(world: &World) -> Option<std::sync::Arc<StreetWorld>> {
    world.street.as_ref().map(|s| s.world.clone())
}

fn name_of(sw: &StreetWorld, e: &Entry) -> String {
    sw.city.segments.get(e.road).map(|s| s.street_name.clone()).unwrap_or_default()
}

/// The station of `e` nearest `p`, and the distance.
fn station_on(e: &Entry, p: DVec2) -> (f64, f64) {
    let mut best = (0.0, f64::MAX);
    for i in 0..e.points.len().saturating_sub(1) {
        let (a, b) = (DVec2::from(e.points[i]), DVec2::from(e.points[i + 1]));
        let ab = b - a;
        let l2 = ab.length_squared();
        if l2 < 1e-9 {
            continue;
        }
        let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
        let d = (a + ab * t).distance(p);
        if d < best.1 {
            best = (e.cum[i] + t * (e.cum[i + 1] - e.cum[i]), d);
        }
    }
    best
}

/// The run of a named street nearest `p`: (entry, station, distance).
fn nearest_run(sw: &StreetWorld, name: &str, p: DVec2) -> Option<(usize, f64, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (ei, e) in sw.plan.entries.iter().enumerate() {
        if name_of(sw, e) != name {
            continue;
        }
        let (s, d) = station_on(e, p);
        if best.is_none_or(|b| d < b.2) {
            best = Some((ei, s, d));
        }
    }
    best
}

/// Centre, unit tangent (growing s) and unit normal (+n) of a run at a station.
fn frame_at(e: &Entry, s: f64) -> (DVec2, DVec2, DVec2) {
    let f = e.frame(s, false);
    (DVec2::new(f.x, f.z), DVec2::new(f.tx, f.tz), DVec2::new(f.nx, f.nz))
}

/// The distance from the centreline to the kerb on `side` (+1 the +n side):
/// the first paving out from the carriageway that is kerb top or footway.
fn kerb_u(sw: &StreetWorld, e: &Entry, s: f64, side: f64) -> Option<f64> {
    let (c, _, n) = frame_at(e, s);
    let mut u = (e.half - 3.0).max(0.0);
    while u < e.half + 6.0 {
        let p = c + n * side * u;
        if matches!(sw.paved_layer_at(p.x, p.y), Some(Layer::CurbTop | Layer::Sidewalk)) {
            return Some(u);
        }
        u += 0.05;
    }
    None
}

/// Feet on the drawn footway `back` metres past the kerb on `side`, and the kerb's distance from the centreline.
fn footway_feet(sw: &StreetWorld, e: &Entry, s: f64, side: f64, back: f64) -> Option<(DVec3, f64)> {
    let k = kerb_u(sw, e, s, side)?;
    let (c, _, n) = frame_at(e, s);
    let p = c + n * side * (k + back);
    if !matches!(sw.paved_layer_at(p.x, p.y), Some(Layer::Sidewalk | Layer::CurbTop | Layer::Ramp)) || sw.buildings.contains(p.x, p.y) {
        return None;
    }
    // a median (the divided boulevards' middle strip) has another carriageway
    // just beyond it: not a footway a pedestrian card stands on.
    if (1..=MEDIAN_PROBE_M as usize).any(|d| {
        let q = p + n * side * d as f64;
        matches!(sw.paved_layer_at(q.x, q.y), Some(Layer::Carriageway))
    }) {
        return None;
    }
    Some((DVec3::new(p.x, sw.surface_at(p.x, p.y, None), p.y), k))
}

/// The footway's width on `side` (`walks[0]` is the +n side).
fn walk(e: &Entry, side: f64) -> f64 {
    if side > 0.0 { e.walks[0] } else { e.walks[1] }
}

/// From the point of a street nearest `from`, `dist` metres along it away from
/// `from`'s side: the run, the station and the unit direction away.
fn along(sw: &StreetWorld, name: &str, from: DVec2, dist: f64) -> Option<(usize, f64, DVec2)> {
    let (e0, s0, _) = nearest_run(sw, name, from)?;
    let e = &sw.plan.entries[e0];
    let (c, t, _) = frame_at(e, s0);
    // away from `from`: toward the run's end that is farther from it (the
    // nearest point is often a run's end, so both sides are looked at).
    let (behind, ahead) = (frame_at(e, (s0 - 5.0).max(0.0)).0, frame_at(e, (s0 + 5.0).min(e.length)).0);
    let dir = if ahead.distance(from) >= behind.distance(from) { t } else { -t };
    let (ei, s, _) = nearest_run(sw, name, c + dir * dist)?;
    let t2 = frame_at(&sw.plan.entries[ei], s).1;
    Some((ei, s, if t2.dot(dir) >= 0.0 { t2 } else { -t2 }))
}

fn h3(v: DVec2) -> DVec3 {
    DVec3::new(v.x, 0.0, v.y)
}

/// An eye 1.7 m over `feet` looking along `dir` (horizontal) at `pitch` radians, the game camera's lens.
fn eye_camera(feet: DVec3, dir: DVec2, pitch: f64, args: &Args, aspect: f32) -> Camera {
    let eye = feet + DVec3::Y * EYE_HEIGHT_M as f64;
    Camera { far: args.far, ..Camera::new(eye.as_vec3(), yaw_of(h3(dir).as_vec3()), pitch as f32, aspect) }
}

fn rotate(v: DVec2, toward: DVec2, deg: f64) -> DVec2 {
    let (s, c) = deg.to_radians().sin_cos();
    (v * c + toward * s).normalize()
}

/// A card's still: where, what, under which look; or why the world could not place it.
struct Card {
    number: u8,
    name: &'static str,
    hour: f64,
    weather: Kind,
    place: Result<(Camera, String, [f64; 3]), String>,
    /// What the card needs that this run lacks.
    unmet: Vec<String>,
    extras: Vec<eo_agents::GpuAgent>,
    hide_near: Vec<([f32; 3], f32)>,
    sub: Option<&'static str>,
}

fn focus_of(cam: &Camera) -> [f64; 3] {
    AgentRuntime::focus_of(cam.eye, cam.forward())
}

/// Card 1 (and 6): Market Street's footway `MARKET_STAND_M` up from The Embarcadero, looking up the street.
fn market(world: &World, args: &Args, aspect: f32) -> Result<(Camera, String, [f64; 3]), String> {
    let sw = street(world).ok_or("no street world (--no-streets)")?;
    let (ei, s, dir) = along(&sw, "Market Street", ferry(), MARKET_STAND_M * gs()).ok_or("Market Street not found near the Ferry Building")?;
    let e = &sw.plan.entries[ei];
    let mut sides = [1.0, -1.0];
    sides.sort_by(|a, b| walk(e, *b).total_cmp(&walk(e, *a))); // the wider footway first
    for side in sides {
        let back = (walk(e, side) * 0.35).clamp(1.0, 3.0);
        if let Some((feet, _)) = footway_feet(&sw, e, s, side, back) {
            let cam = eye_camera(feet, dir, 0.0, args, aspect);
            let what = format!("Market Street's footway {:.0} m up from The Embarcadero ({:.1} m from the kerb), eye {EYE_HEIGHT_M} m, looking up Market Street away from the Ferry Building", MARKET_STAND_M * gs(), back);
            return Ok((cam, what, focus_of(&cam)));
        }
    }
    Err("no footway beside Market Street there".into())
}

/// Card 2: the signalised, painted junction of The Embarcadero and a named
/// street nearest the Ferry Building (Market Street's first), from the corner
/// footway at the end of The Embarcadero's crosswalk, looking along it.
fn crosswalk(world: &World, args: &Args, aspect: f32) -> Result<(Camera, String, [f64; 3]), String> {
    let sw = street(world).ok_or("no street world (--no-streets)")?;
    let fb = ferry();
    let mut best: Option<(f64, usize)> = None;
    for (ni, node) in sw.plan.nodes.iter().enumerate() {
        if !node.earns_paint || node.approaches.len() < 3 {
            continue;
        }
        let names: Vec<String> = node.approaches.iter().map(|a| name_of(&sw, &sw.plan.entries[a.entry])).collect();
        // a street with a name crossing it (not a service road's survey label).
        let crossed = names.iter().any(|n| n != "The Embarcadero" && !n.is_empty() && !n.starts_with("SF Rd"));
        if !names.iter().any(|n| n == "The Embarcadero") || !crossed || !node.signalised() {
            continue;
        }
        let d = DVec2::from(node.position).distance(fb) - if names.iter().any(|n| n == "Market Street") { 1000.0 } else { 0.0 };
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, ni));
        }
    }
    let (_, ni) = best.ok_or("no signalised, painted junction of The Embarcadero and a named street")?;
    let node = &sw.plan.nodes[ni];
    let np = DVec2::from(node.position);
    let names: Vec<String> = node.approaches.iter().map(|a| name_of(&sw, &sw.plan.entries[a.entry])).collect();
    for a in node.approaches.iter().filter(|a| name_of(&sw, &sw.plan.entries[a.entry]) == "The Embarcadero") {
        let e = &sw.plan.entries[a.entry];
        let u = DVec2::from(a.u);
        let (s, _) = station_on(e, np + u * (node.band_depth + 1.0));
        let (c, _, n) = frame_at(e, s);
        // the corner on the Ferry Building's side first.
        let mut sides = [1.0, -1.0];
        sides.sort_by(|x, y| (c + n * *x * 10.0).distance(fb).total_cmp(&(c + n * *y * 10.0).distance(fb)));
        for side in sides {
            if let Some((feet, _)) = footway_feet(&sw, e, s, side, 1.2) {
                // across the carriageway along the crosswalk, turned 10 degrees toward the junction.
                let dir = rotate(-n * side, -u, 10.0);
                let cam = eye_camera(feet, dir, -0.02, args, aspect);
                let what = format!("the junction of {} at ({:.0}, {:.0}), {:.0} m from the Ferry Building, from the corner footway {:.1} m from its centre (the crossing band is {:.1} m deep), eye {EYE_HEIGHT_M} m, looking along The Embarcadero's crosswalk", names.join(", "), np.x, np.y, np.distance(fb), node.band_depth + 1.0, node.band_depth);
                return Ok((cam, what, [np.x, np.y, focus_of(&cam)[2]]));
            }
        }
    }
    Err("no corner footway at the junction".into())
}

/// Card 3: California Street's footway `CALIFORNIA_STAND_M` from its east end, looking up the street, raised 6 degrees.
fn canyon(world: &World, args: &Args, aspect: f32) -> Result<(Camera, String, [f64; 3]), String> {
    let sw = street(world).ok_or("no street world (--no-streets)")?;
    let (ei, s, dir) = along(&sw, "California Street", ferry(), CALIFORNIA_STAND_M * gs()).ok_or("California Street not found near the Ferry Building")?;
    let e = &sw.plan.entries[ei];
    for side in [1.0, -1.0] {
        if let Some((feet, _)) = footway_feet(&sw, e, s, side, (walk(e, side) * 0.35).clamp(1.0, 3.0)) {
            let cam = eye_camera(feet, dir, 6f64.to_radians(), args, aspect);
            let what = format!("California Street's footway {:.0} m from its east end (among the Embarcadero Center and Financial District towers), eye {EYE_HEIGHT_M} m, looking up the street, raised 6 degrees", CALIFORNIA_STAND_M * gs());
            return Ok((cam, what, focus_of(&cam)));
        }
    }
    Err("no footway beside California Street there".into())
}

/// Card 4: the water bench's Embarcadero promenade view (south-east of the Ferry Building, out over the bay).
fn waterfront(world: &World, args: &Args, aspect: f32) -> Result<(Camera, String, [f64; 3]), String> {
    let (_, cam, what) = water_bench::capture_cameras(world, aspect, args.far).into_iter().find(|c| c.0 == "embarcadero").ok_or("the Embarcadero promenade is not found (shore field or road missing)")?;
    Ok((cam, what, focus_of(&cam)))
}

/// Card 5: The Embarcadero's footway on the Ferry Building's side, `DRIZZLE_BACK_M` south-east of it, looking north-west along the road.
fn wet_street(world: &World, args: &Args, aspect: f32) -> Result<(Camera, String, [f64; 3]), String> {
    let sw = street(world).ok_or("no street world (--no-streets)")?;
    let fb = ferry();
    let (ei, s0, _) = nearest_run(&sw, "The Embarcadero", fb).ok_or("The Embarcadero not found")?;
    let e = &sw.plan.entries[ei];
    let (c, t, n) = frame_at(e, s0);
    let nw = DVec2::new(-1.0, -1.0).normalize();
    let sign = if t.dot(nw) >= 0.0 { 1.0 } else { -1.0 };
    let s = (s0 - sign * DRIZZLE_BACK_M * gs()).clamp(0.0, e.length);
    let side_fb = if (fb - c).dot(n) >= 0.0 { 1.0 } else { -1.0 };
    for side in [side_fb, -side_fb] {
        if let Some((feet, _)) = footway_feet(&sw, e, s, side, 2.0) {
            let dir = rotate(t * sign, -n * side, 10.0);
            let cam = eye_camera(feet, dir, -0.03, args, aspect);
            let what = format!("The Embarcadero's footway {} the Ferry Building, {:.0} m south-east of it, 2 m from the kerb, eye {EYE_HEIGHT_M} m, looking north-west along the road", if side == side_fb { "on the side of" } else { "across from" }, DRIZZLE_BACK_M * gs());
            return Ok((cam, what, focus_of(&cam)));
        }
    }
    Err("no footway beside The Embarcadero there".into())
}

/// A card 7 sub-frame: its name, its place, the character's instance and its feet.
type SubFrame = (&'static str, Result<(Camera, String, [f64; 3]), String>, Vec<eo_agents::GpuAgent>, [f32; 3]);

/// Card 7: a third-person character on The Embarcadero's city-side footway
/// across from the Ferry Building, facing the road: standing, walking toward
/// the kerb, stopped at it; the on-foot camera rig behind it (eo-player's rest
/// boom, lens height and shoulder offset).
fn character(world: &World, io_lib: Option<&eo_agents::library::Library>, args: &Args, aspect: f32) -> Vec<SubFrame> {
    use eo_player::camera::on_foot;
    let subs = ["7a-standing", "7b-walking", "7c-stopped-at-kerb"];
    let fail = |why: &str| -> Vec<SubFrame> { subs.iter().map(|s| (*s, Err(why.to_string()), vec![], [0.0; 3])).collect() };
    let Some(sw) = street(world) else { return fail("no street world (--no-streets)") };
    let fb = ferry();
    let Some((ei, s, _)) = nearest_run(&sw, "The Embarcadero", fb) else { return fail("The Embarcadero not found") };
    let e = &sw.plan.entries[ei];
    let (c, _, n) = frame_at(e, s);
    let side_fb = if (fb - c).dot(n) >= 0.0 { 1.0 } else { -1.0 };
    let mut out = vec![];
    for side in [-side_fb, side_fb] {
        let Some(k) = kerb_u(&sw, e, s, side) else { continue };
        let face = -n * side; // toward the road (and the Ferry Building across it)
        let f3 = h3(face);
        let right = DVec3::new(-f3.z, 0.0, f3.x);
        let (d, el) = (on_foot::walk_distance(), on_foot::rest_elevation());
        for (i, sub) in subs.iter().enumerate() {
            let back = CHARACTER_BACK_M[i];
            let p = c + n * side * (k + back);
            if !matches!(sw.paved_layer_at(p.x, p.y), Some(Layer::Sidewalk | Layer::CurbTop | Layer::Ramp)) {
                out.clear();
                break;
            }
            let feet = DVec3::new(p.x, sw.surface_at(p.x, p.y, None), p.y);
            let pivot = feet + DVec3::Y * on_foot::PIVOT_HEIGHT;
            let eye = pivot - f3 * d * el.cos() + DVec3::Y * d * el.sin() + right * on_foot::SHOULDER;
            let cam = Camera { far: args.far, fov_y_deg: on_foot::WALK_FOV as f32, ..Camera::new(eye.as_vec3(), yaw_of(f3.as_vec3()), -el as f32, aspect) };
            let (speed, phase) = if i == 1 { (eo_player::tuning::speed::WALK_MPS, 0.3) } else { (0.0, 0.0) };
            let body = io_lib.and_then(|l| eo_agents::feed::player_body(l, feet.to_array(), face.x.atan2(face.y), speed, phase, 0.0));
            let what = format!(
                "a third-person character on The Embarcadero's footway {} the Ferry Building, facing the road, {} ({back} m from the kerb edge, {:.1} m from the centreline), the on-foot camera rig behind it: boom {d:.2} m at {:.1} degrees over the pivot ({} m), shoulder {} m, lens {} degrees",
                if side == side_fb { "on the side of" } else { "across from" },
                ["standing", "walking toward the kerb", "stopped at the kerb"][i],
                k,
                el.to_degrees(),
                on_foot::PIVOT_HEIGHT,
                on_foot::SHOULDER,
                on_foot::WALK_FOV
            );
            out.push((*sub, Ok((cam, what, [feet.x, feet.z, face.x.atan2(face.y)])), body.into_iter().collect(), feet.as_vec3().to_array()));
        }
        if out.len() == subs.len() {
            return out;
        }
    }
    fail("no kerb with a footway beside The Embarcadero there")
}

/// The first card's camera (streamed in before the run).
pub fn first_camera(world: &World, args: &Args, aspect: f32) -> Option<Camera> {
    market(world, args, aspect).or_else(|_| waterfront(world, args, aspect)).ok().map(|p| p.0)
}

/// A fall-back when a card's place is not found: the Ferry Building from the promenade view.
fn fallback(world: &World, args: &Args, aspect: f32) -> Option<(Camera, [f64; 3])> {
    waterfront(world, args, aspect).ok().map(|p| (p.0, p.2))
}

pub fn run(io: &mut Io, dir: &Path, timeout: f64) -> Result<()> {
    let aspect = io.aspect();
    let (world, args) = (io.world, io.args);
    let people = io.people_drawable();
    let agents_on = io.agents.opts.on;
    let lib = io.agents.lib.clone();
    let no_people = "no pedestrians: the optional people assets (EO_PEOPLE_ROOT, EO_PEOPLE_MOTIONS_ROOT, EO_PEOPLE_STYLES_ROOT) are absent or do not match their manifests, or --no-rocketbox; the simulation's people are not drawn".to_string();
    let no_traffic = "no traffic: --no-agents".to_string();
    let mut cards: Vec<Card> = vec![];
    let card = |number: u8, name: &'static str, hour: f64, weather: Kind, place, unmet: Vec<String>| Card { number, name, hour, weather, place, unmet, extras: vec![], hide_near: vec![], sub: None };
    let market_place = market(world, args, aspect);
    cards.push(card(1, "card1-street-day", DAY_H, Kind::Clear, market_place.clone(), if people { vec![] } else { vec![format!("{no_people} (the street has none)")] }));
    let mut unmet2 = vec![];
    if !agents_on {
        unmet2.push(no_traffic.clone());
    }
    if !people {
        unmet2.push(no_people.clone());
    }
    cards.push(card(2, "card2-crosswalk-traffic", DAY_H, Kind::Clear, crosswalk(world, args, aspect), unmet2));
    cards.push(card(3, "card3-canyon-golden", GOLDEN_H, Kind::Clear, canyon(world, args, aspect), vec![]));
    cards.push(card(4, "card4-waterfront-day", DAY_H, Kind::Clear, waterfront(world, args, aspect), vec![]));
    cards.push(card(5, "card5-wet-drizzle", DRIZZLE_H, Kind::Drizzle, wet_street(world, args, aspect), if args.look.sky == eo_render::sky::SkyOptions::off() { vec!["no weather: --no-sky".into()] } else { vec![] }));
    cards.push(card(6, "card6-night-street", NIGHT_H, Kind::Clear, market_place, if agents_on { vec![] } else { vec![no_traffic.clone()] }));
    for (sub, place, extras, feet) in character(world, lib.as_deref(), args, aspect) {
        let mut unmet = vec![];
        if extras.is_empty() {
            unmet.push(format!("no character: the player's body is the first pool person and the agent library has none ({}); the camera rig is rendered without it", if people { "the body was not built" } else { "the optional people assets are absent" }));
        }
        cards.push(Card { number: 7, name: sub, hour: DAY_H, weather: Kind::Clear, place, unmet, extras, hide_near: vec![(feet, 0.8)], sub: Some(sub) });
    }
    let common = shots::common_json(io, "scene-cards");
    let mut written = vec![];
    for c in cards {
        let name = if c.sub.is_some() { format!("card{}", c.name) } else { c.name.to_string() };
        if !shots::wanted(&name) {
            continue;
        }
        let mut unmet = c.unmet.clone();
        let (cam, what, focus) = match &c.place {
            Ok(p) => p.clone(),
            Err(why) => {
                let Some((cam, focus)) = fallback(world, args, aspect) else {
                    eprintln!("scene-cards: {name}: {why}, and no fall-back view; skipped");
                    continue;
                };
                unmet.push(format!("the card's place was not found ({why}); the Embarcadero promenade view is drawn instead"));
                (cam, "fall-back: the Embarcadero promenade view".to_string(), focus)
            }
        };
        let gf = eo_data::geo::GeoFrame::of_city(&world.city.meta);
        let mut info = Map::new();
        info.insert(
            "card".into(),
            json!({
                "number": c.number, "category": CATEGORIES[c.number as usize - 1], "sub_frame": c.sub,
                "place": what, "honoured": unmet.is_empty(), "unmet": unmet,
                "corridor": "the Ferry Building / Embarcadero hero corridor (Docs/VISUAL_QUALITY_GATE.md, minimum next quality milestone)",
            }),
        );
        info.insert("resolved".into(), shots::pose_json(&gf, world, &cam, None));
        info.insert("character".into(), json!(c.extras.first().map(|a| json!({ "feet": a.pos, "speed_mps": a.speed, "model": a.model }))));
        let shot = Shot { name: name.clone(), cam, hour: c.hour, weather: WeatherMode::Preset(c.weather), focus, info, extras: c.extras.clone(), notes: unmet.clone(), photo: None, subject: None };
        io.agents.hide_near = c.hide_near.clone();
        shots::shoot(io, &common, &shot, dir, timeout)?;
        io.agents.hide_near.clear();
        written.push(name);
    }
    if shots::wanted("card8-traversal") {
        traversal(io, dir, timeout)?;
        written.push("card8-traversal".into());
    }
    println!("scene-cards: {} written to {}: {}", written.len(), dir.display(), written.join(", "));
    Ok(())
}

/// Card 8: the promenade south-east of the Ferry Building at the player's run,
/// every frame drawn at 60 Hz of wall time (streaming, traffic, water and
/// clouds running), `--card8-fps` of them written as a numbered sequence,
/// placed so that a 128 m tile boundary is crossed halfway.
fn traversal(io: &mut Io, dir: &Path, timeout: f64) -> Result<()> {
    let seconds = shots::arg_f64("--card8-seconds")?.unwrap_or(30.0).clamp(1.0, 600.0);
    let fps = shots::arg_f64("--card8-fps")?.unwrap_or(10.0).clamp(1.0, 60.0);
    let speed = shots::arg_f64("--card8-speed")?.unwrap_or(eo_player::tuning::speed::RUN_MPS).clamp(0.1, 60.0);
    let every = (60.0 / fps).round().max(1.0) as usize;
    let frames = (seconds * 60.0).round() as usize;
    let length = speed * seconds;
    let (world, args, aspect) = (io.world, io.args, io.aspect());
    let mut notes: Vec<String> = vec![];
    // The road's line (both carriageways averaged, ordered south-east) from
    // the Ferry Building on: a straight line fitted to that stretch (The
    // Embarcadero runs straight to Rincon Park; the averaged points jump
    // between its carriageways and ramps by metres), moved toward the water
    // onto the promenade: as far as it can go everywhere along the stretch
    // and stay 6 m inland of the shore (the smallest of the promenade points' offsets).
    let (road, axis) = water_bench::embarcadero(world);
    if road.len() < 2 {
        return Err(anyhow!("card 8: The Embarcadero is not found; no traversal"));
    }
    let total: f64 = road.windows(2).map(|w| w[0].distance(w[1]) as f64).sum();
    let fb = Vec3::new(ferry().x as f32, 0.0, ferry().y as f32);
    let s_start = water_bench::arc_nearest(&road, fb) as f64 + TRAVERSE_FROM_SE_M * gs();
    let s_end = (s_start + TRAVERSE_STRETCH_M * gs()).min(total);
    let samples: Vec<DVec3> = (0..)
        .map(|k| s_start + 10.0 * k as f64)
        .take_while(|s| *s <= s_end)
        .map(|s| water_bench::polyline_at(&road, s as f32).0.as_dvec3())
        .collect();
    if samples.len() < 4 {
        return Err(anyhow!("card 8: The Embarcadero has no {:.0} m stretch south-east of the Ferry Building", TRAVERSE_STRETCH_M * gs()));
    }
    let mean = samples.iter().fold(DVec3::ZERO, |a, p| a + *p) / samples.len() as f64;
    let (mut sxx, mut sxz, mut szz) = (0.0, 0.0, 0.0);
    for p in &samples {
        let d = *p - mean;
        sxx += d.x * d.x;
        sxz += d.x * d.z;
        szz += d.z * d.z;
    }
    let ang = 0.5 * (2.0 * sxz).atan2(sxx - szz);
    let mut along_dir = DVec3::new(ang.cos(), 0.0, ang.sin());
    if along_dir.dot(axis.as_dvec3()) < 0.0 {
        along_dir = -along_dir;
    }
    let normal = DVec3::new(-along_dir.z, 0.0, along_dir.x);
    let project = |p: DVec3| mean + along_dir * (p - mean).dot(along_dir);
    let (line_a, line_len) = (project(samples[0]), (project(samples[samples.len() - 1]) - project(samples[0])).dot(along_dir));
    // Toward the water: the side the shore field says is wetter 120 m out.
    let wet = |p: DVec3| world.water.as_ref().map(|w| w.field.signed_at(p.x, p.z)).unwrap_or(f64::MIN);
    let mid = line_a + along_dir * (line_len * 0.5);
    let side = if wet(mid + normal * 120.0) >= wet(mid - normal * 120.0) { 1.0 } else { -1.0 };
    let n_w = normal * side;
    // At every 5 m: past the last carriageway toward the water by
    // `TRAVERSE_KERB_CLEAR_M`, and no nearer the shore than that again (the
    // promenade between the boulevard and the seawall); smoothed over +-25 m.
    let sw = street(world);
    let mut offs = vec![];
    for k in 0..=((line_len / 5.0) as usize) {
        let p = line_a + along_dir * (5.0 * k as f64);
        let (mut last_road, mut shore) = (None, None);
        let mut u = 0.0;
        while u < 160.0 {
            let q = p + n_w * u;
            if sw.as_ref().is_some_and(|sw| matches!(sw.paved_layer_at(q.x, q.z), Some(Layer::Carriageway | Layer::CurbFace))) {
                last_road = Some(u);
            }
            if wet(q) > -TRAVERSE_KERB_CLEAR_M {
                shore = Some(u);
                break;
            }
            u += 0.5;
        }
        let Some(shore) = shore else { continue };
        let want = last_road.map(|r| r + TRAVERSE_KERB_CLEAR_M).unwrap_or(shore * 0.5);
        offs.push((5.0 * k as f64, want.min(shore - TRAVERSE_KERB_CLEAR_M).max(0.0), want > shore - TRAVERSE_KERB_CLEAR_M));
    }
    if offs.len() < 4 {
        return Err(anyhow!("card 8: no promenade beside The Embarcadero south-east of the Ferry Building"));
    }
    let squeezed = offs.iter().filter(|o| o.2).count();
    if squeezed > 0 {
        notes.push(format!("at {squeezed} of {} stations the promenade is narrower than {} m past the carriageway and {} m from the shore; the path runs between them", offs.len(), TRAVERSE_KERB_CLEAR_M, TRAVERSE_KERB_CLEAR_M));
    }
    let smooth: Vec<(f64, f64)> = (0..offs.len()).map(|i| {
        let w: Vec<f64> = offs.iter().filter(|o| (o.0 - offs[i].0).abs() <= 25.0).map(|o| o.1).collect();
        (offs[i].0, w.iter().sum::<f64>() / w.len() as f64)
    }).collect();
    let off_at = |t: f64| -> f64 {
        let i = smooth.partition_point(|o| o.0 <= t).clamp(1, smooth.len() - 1);
        let (a, b) = (smooth[i - 1], smooth[i]);
        a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0)
    };
    let off = smooth.iter().map(|o| o.1).sum::<f64>() / smooth.len() as f64;
    let path: Vec<DVec3> = (0..=(line_len * 2.0) as usize).map(|k| {
        let t = 0.5 * k as f64;
        line_a + along_dir * t + n_w * off_at(t)
    }).collect();
    let arc: Vec<f64> = std::iter::once(0.0).chain(path.windows(2).scan(0.0, |a, w| {
        *a += w[0].distance(w[1]);
        Some(*a)
    })).collect();
    let at = |s: f64| -> DVec3 {
        let i = arc.partition_point(|&a| a <= s).clamp(1, arc.len() - 1);
        let t = ((s - arc[i - 1]) / (arc[i] - arc[i - 1]).max(1e-9)).clamp(0.0, 1.0);
        path[i - 1].lerp(path[i], t)
    };
    let path_len = *arc.last().unwrap_or(&0.0);
    // The first tile boundary at least half the clip in, the clip centred on it.
    let key = |p: DVec3| eo_street::tile_key(p.x, p.z);
    let mut cross = None;
    let mut k0 = key(at(length * 0.5));
    let mut s = length * 0.5;
    while s < path_len - length * 0.5 {
        let k = key(at(s));
        if k != k0 {
            cross = Some(s);
            break;
        }
        k0 = k;
        s += 0.25;
    }
    let a = match cross {
        Some(c) => c - length * 0.5,
        None => {
            notes.push(format!("no 128 m tile boundary within {path_len:.0} m of promenade; the clip starts at the path's start"));
            0.0
        }
    };
    let eye_at = |s: f64| -> DVec3 {
        let p = at(s.clamp(0.0, path_len));
        DVec3::new(p.x, water_bench::ground(world, p.x as f32, p.z as f32) as f64 + EYE_HEIGHT_M as f64, p.z)
    };
    let cam_at = |i: usize| -> Camera {
        let s = a + speed * i as f64 / 60.0;
        let look = (at(s + TRAVERSE_LOOK_M) - at(s - TRAVERSE_LOOK_M)).normalize_or(h3(DVec2::new(axis.x as f64, axis.z as f64)));
        Camera { far: args.far, ..Camera::new(eye_at(s).as_vec3(), yaw_of(look.as_vec3()), -0.03, aspect) }
    };
    // The crossings inside the clip.
    let mut crossings = vec![];
    let mut last = key(cam_at(0).eye.as_dvec3());
    for i in 1..=frames {
        let k = key(cam_at(i).eye.as_dvec3());
        if k != last {
            crossings.push(json!({ "sim_frame": i, "t_s": i as f64 / 60.0, "written_frame": i / every, "from": [last.0, last.1], "to": [k.0, k.1], "eye": cam_at(i).eye.to_array() }));
            last = k;
        }
    }
    if crossings.is_empty() {
        notes.push("the clip crosses no 128 m tile boundary".into());
    }
    for i in (0..=frames).step_by(60) {
        let c = cam_at(i);
        if let Some(b) = shots::inside_building(world, c.eye) {
            notes.push(format!("at {:.0} s the eye is inside building {}", i as f64 / 60.0, b["building"]));
        }
        if let Some(w) = world.water.as_ref().map(|w| w.field.signed_at(c.eye.x as f64, c.eye.z as f64)).filter(|d| *d > 0.0) {
            notes.push(format!("at {:.0} s the eye is {w:.1} m out over the water", i as f64 / 60.0));
        }
    }
    let seq = dir.join("card8-traversal");
    std::fs::create_dir_all(&seq)?;
    // Settle, traffic and a converged first frame at the start, still.
    let cam0 = cam_at(0);
    eprintln!("scene-cards: card8-traversal: {frames} frames at 60 Hz, {length:.0} m at {speed} m/s from ({:.0}, {:.0}), {} tile crossings, writing every {every}", cam0.eye.x, cam0.eye.z, crossings.len());
    io.set_look(DAY_H, WeatherMode::Preset(Kind::Clear));
    let agents = io.place_agents(focus_of(&cam0));
    let settle = io.settle(&cam0, timeout)?;
    for _ in 0..48 {
        draw(io.ctx, io.renderer, io.attach, io.resolve, &io.targets.depth, &cam0);
    }
    // The PNGs are written on threads of their own so the frames keep their 60 Hz slots.
    let (w, h) = (io.color.width(), io.color.height());
    let (tx, rx) = std::sync::mpsc::sync_channel::<(std::path::PathBuf, Vec<u8>)>(8);
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let writers: Vec<_> = (0..4)
        .map(|_| {
            let rx = rx.clone();
            std::thread::spawn(move || -> Result<()> {
                loop {
                    let job = rx.lock().map_err(|_| anyhow!("writer lock"))?.recv();
                    let Ok((p, rgba)) = job else { return Ok(()) };
                    write_png(&p, w, h, &rgba)?;
                }
            })
        })
        .collect();
    let base = args.look.sky.world_time_s;
    io.renderer.look.sky.set_clock(base, 1.0 / 60.0); // the clouds move with the clip
    let view_t0 = super::view_bench::clock();
    let t0 = Instant::now();
    let mut subs: Vec<eo_render::wgpu::SubmissionIndex> = Vec::with_capacity(frames + 1);
    let mut records = vec![];
    let mut late_max = 0f64;
    for i in 0..=frames {
        let due = t0 + Duration::from_secs_f64(i as f64 / 60.0);
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        let late = Instant::now().saturating_duration_since(due).as_secs_f64() * 1e3;
        late_max = late_max.max(late);
        if i >= 2 {
            io.ctx.device.poll(eo_render::wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }).map_err(|e| anyhow!("{e}"))?;
        }
        let cam = cam_at(i);
        io.renderer.observe_view(&cam, Some(view_t0 + i as f64 / 60.0));
        io.models.set_camera(cam.eye);
        let pf = io.models.pump(io.ctx, io.renderer, io.world, cam.eye)?;
        if io.props.on {
            io.props.pump(io.ctx, io.renderer, &mut io.models.uploads, cam.eye, Some(io.agents.sim_time), 1.0 / 60.0);
            io.agents.parked = io.props.parked_agents(io.agents.lib.as_deref(), cam.eye).to_vec();
        }
        io.agents.pump(io.ctx, io.renderer, focus_of(&cam), 1.0 / 60.0);
        if let Some(wt) = io.renderer.water.get() {
            wt.set_time(WATER_CLOCK_S + i as f64 / 60.0);
        }
        io.renderer.update(&io.ctx.queue, &cam);
        let mut enc = io.ctx.device.create_command_encoder(&Default::default());
        io.renderer.encode(&mut enc, io.attach, io.resolve, &io.targets.depth, None);
        subs.push(eo_render::pace::submit(&io.ctx.queue, [enc.finish()]));
        if i % every == 0 {
            let n = i / every;
            let rgba = read_color(io.ctx, io.color)?;
            let p = seq.join(format!("{n:04}.png"));
            tx.send((p, rgba)).map_err(|_| anyhow!("the PNG writers stopped"))?;
            let k = key(cam.eye.as_dvec3());
            records.push(json!({
                "frame": n, "file": format!("card8-traversal/{n:04}.png"), "sim_frame": i, "t_s": i as f64 / 60.0,
                "eye": cam.eye.to_array(), "yaw_rad": cam.yaw, "pitch_rad": cam.pitch, "tile": [k.0, k.1],
                "uploads_pending": io.models.uploads.pending(), "streaming_settled": io.models.settled(), "bytes_uploaded": pf.bytes, "late_ms": late,
            }));
        }
    }
    io.ctx.device.poll(eo_render::wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    drop(tx);
    for wr in writers {
        wr.join().map_err(|_| anyhow!("a PNG writer panicked"))??;
    }
    super::view_bench::set_clock(view_t0 + frames as f64 / 60.0);
    io.renderer.look.sky.set_clock(base, 0.0);
    let achieved_fps = frames as f64 / t0.elapsed().as_secs_f64().max(1e-9);
    if late_max > 50.0 {
        notes.push(format!("the frames ran at {achieved_fps:.1} Hz, not 60 (the last started {late_max:.0} ms after its slot): the camera moves by the frame index, so the path and the frames are exact, but streaming and the other threads had {:.1} x the wall time per metre that 60 Hz would give them", 60.0 / achieved_fps));
    }
    if !io.people_drawable() {
        notes.push("no pedestrians: the optional people assets are absent".into());
    }
    let gf = eo_data::geo::GeoFrame::of_city(&world.city.meta);
    let mut m = shots::common_json(io, "scene-cards");
    m.insert("name".into(), json!("card8-traversal"));
    m.insert(
        "card".into(),
        json!({
            "number": 8, "category": CATEGORIES[7], "honoured": !crossings.is_empty(), "unmet": if crossings.is_empty() { vec!["no tile boundary crossed"] } else { vec![] },
            "place": format!("the Embarcadero promenade south-east of the Ferry Building: a straight line fitted to The Embarcadero's line over {:.0} m, on average {off:.1} m from it toward the water ({TRAVERSE_KERB_CLEAR_M} m past the carriageway, {TRAVERSE_KERB_CLEAR_M} m inland of the shore, smoothed over 50 m), eye {EYE_HEIGHT_M} m over the drawn surface, looking along the path", TRAVERSE_STRETCH_M * gs()),
            "corridor": "the Ferry Building / Embarcadero hero corridor (Docs/VISUAL_QUALITY_GATE.md, minimum next quality milestone)",
        }),
    );
    m.insert(
        "sequence".into(),
        json!({
            "seconds": seconds, "speed_mps": speed, "length_m": length, "sim_fps": 60, "written_fps": 60.0 / every as f64, "written_frames": records.len(),
            "start": cam0.eye.to_array(), "end": cam_at(frames).eye.to_array(), "start_resolved": shots::pose_json(&gf, world, &cam0, None),
            "pacing": "every frame waits for its 1/60 s slot of wall time; the camera is placed by the frame index; streaming, traffic, props, water and clouds run as in play",
            "tile_m": eo_street::STREET_TILE_M, "tile_crossings": crossings, "latest_start_ms": late_max, "achieved_fps": achieved_fps,
        }),
    );
    m.insert("time_of_day".into(), json!(DAY_H));
    m.insert("weather".into(), json!("clear"));
    m.insert("sky".into(), super::sky_shots::describe(io.renderer));
    m.insert("agents".into(), agents);
    m.insert("settle_at_start".into(), settle);
    m.insert("frames".into(), json!(records));
    m.insert("notes".into(), json!(notes));
    let mp = dir.join("card8-traversal.json");
    std::fs::write(&mp, serde_json::to_string_pretty(&Value::Object(m))?)?;
    println!("wrote {} frames to {} and {}{}", records.len(), seq.display(), mp.display(), if notes.is_empty() { String::new() } else { format!("; notes: {}", notes.join("; ")) });
    Ok(())
}
