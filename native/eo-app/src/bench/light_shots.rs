//! The lighting reference cards (`--bench --light-shots --out DIR --label
//! NAME`): fixed cameras at a fixed hour under the clear preset, captured now
//! so the lighting stages (ambient volume, far shadows, lamp and headlight
//! shadows, lit fog, the drawn hour following the clock) have a matched
//! "before" to compare against:
//!
//! * `alley`: Maiden Lane (a 3 m downtown lane between tall buildings), eye
//!   1.7 m, looking along it, at noon; `alley-low-sun` the same at golden hour;
//! * `plaza`: the landmarks card `plaza` (Justin Herman Plaza from its
//!   south-east corner, eye 1.7 m) at noon; `plaza-low-sun` at golden hour;
//! * `skyline-low-sun`: the far-city card `towers-roof` (35 m over Market
//!   Street, looking at the towers) at golden hour;
//! * `hill-shadow`: the far-city card `twin-peaks-500m` (over Twin Peaks,
//!   looking at downtown, the hill's shadow ahead) at golden hour;
//! * `parked-row-night`, `junction-night`: the props cards `parked` and
//!   `junction` at night;
//! * `under-viaduct`: 5th Street between Harrison and Bryant where it passes
//!   nearest the I-80 viaduct's line (the nearest grade-separated motorway
//!   way), eye 1.7 m, looking along the viaduct, at noon;
//!   `under-viaduct-low-sun` at golden hour (the three pairs are the ambient
//!   volume's, `--ambient-volume`: narrow and covered against open);
//! * `dusk-HHMM`: the kit street (the sky cards' frame 260) at eight hours
//!   from golden hour into the night (a strip with no step expected);
//! * `far-city-overview`, `far-elevated-sweep-0`, `far-elevated-sweep-q`:
//!   the `city_overview` path's first frame and the `elevated_sweep` path's
//!   first and quarter frames at golden hour (the far sun shadow maps'
//!   cards, `--far-shadows`, with the skyline and the hill).
//!
//! Output `capture-light-<label>-<name>.png` and `capture-light-<label>.json`
//! (every card's eye, hour and what it shows); `EO_LIGHT_SHOTS=a,b` keeps
//! the cards whose name contains one of them; `EO_LIGHT_SHOTS_HOUR=H` draws
//! every card at hour H instead of its own (recorded in the JSON). With `--no-sky` only the hour
//! moves (as the sky cards). A card whose place is not found is left out
//! with a line saying so.
//!
//! `--bench --dusk-strip --out DIR --label NAME` (`run_strip`): the drawn hour
//! moving at one fixed camera (the kit street's frame 260, sky in view), 40
//! frames a clock minute apart across sunset and 40 across sunrise. With
//! `--smooth-time` each frame is reached through the soft path from the one
//! before (`STRIP_SUBSTEPS` frames, the clock stepping through the minute);
//! without it, through the hard path, one `set_time_of_day` a frame and the
//! same frames rendered. The world clock is pinned at the strip's start, so
//! both runs draw the same clouds. Output `capture-light-<label>-dusk-<nn>.png`
//! and `-dawn-<nn>.png`, a CSV each (`capture-light-<label>-dusk.csv`: mean
//! luminance per frame and the step from the frame before, so a step is a
//! number) and `capture-light-<label>-strips.json`. `EO_DUSK_STRIP=dusk` or
//! `dawn` runs one.
//!
//! `--bench --light-shots` with `EO_PROBE_STRIP=path[,frames[,metres a
//! frame[,turn]]]` (`run_probe_strip`, in place of the cards): the
//! reflections while the eye moves along a street and probe captures finish
//! (`--probe-grid`, or the single probe without it). From 45 % along the path
//! (default `kit_street`, Castro Street's shops) the eye moves `metres a
//! frame` (default 0.5: 30 m/s at 60 fps, so the single probe's 16 m and the
//! grid's 32 m anchors are passed several times) looking 45 degrees across the
//! street to the `turn` side (default -1, the `kit-shop` card's), one frame
//! drawn a step (no settle frames between), the world clock pinned, for
//! `frames` frames (default 120). Every frame is read back:
//! `capture-light-<label>-probe-strip.csv` has per frame the single probe's
//! and the grid's finished captures, the mean luminance and the largest
//! change of a 16 x 9 cell's mean display value from the frame before; the
//! frames before and after each finished capture (and, with the grid, its
//! fade's middle and end) are written as `-probe-strip-fNNN.png`, a summary
//! in `capture-light-<label>-probe-strip.json`, every cell's mean a frame in
//! `-probe-strip-cells.csv`. `EO_PROBE_STRIP_FREEZE=1` holds the probes as
//! captured at the first camera: a reference run, which subtracted from a
//! strip cell by cell leaves what the probes changed (not the camera's motion).
//!
//! The lit fog's cards (`--lit-fog`): `EO_LIGHT_SHOTS_WEATHER=fog|drizzle|...`
//! draws every card under that weather preset instead of clear (recorded in
//! the JSON); `EO_LIGHT_SHOTS` naming `night-kit` adds `night-kit-street-0`,
//! `-mid` and `-260` (the night_kit_street bench path's frames 0, its middle
//! and the sky cards' frame 260, at night; not in the default set).
//! `EO_HEADLIGHT_STRIP=frames` (default 60; `run_headlight_strip`, in place
//! of the cards): traffic runs at the props card `junction`'s street until a
//! car drives past at over `STRIP_CAR_MIN_MS`; the camera stands 11 m beside
//! its path (on the street surface, 2.4 m up, no agent drawn within 5 m of
//! it), looking across it at where the car will be half way through, and every frame of the car crossing the view is written
//! (`capture-light-<label>-headlights-fNN.png`) with the traffic pumped one
//! frame step a frame, at night under the weather (fog by default), with a
//! JSON of the car, the camera and each frame's mean luminance;
//! `EO_HEADLIGHT_STRIP_VIEW=behind` stands 9 m behind the car where it starts,
//! 3 m up, looking along its heading instead.
//!
//! The light shadows' cards (`--light-shadows`, `--headlight-shadows`): each
//! card's JSON carries the atlas and that frame's selection
//! (`light_shadows_json`); `EO_LIGHT_SHADOW_ATLAS=1` writes the atlas beside
//! each card (`capture-light-<label>-<name>-atlas.png`, near black, far
//! white); `EO_LIGHT_SHADOW_MAPS_OFF=1` draws the maps but lets no light read
//! them (the selection, stable clusters, LED colour and occlusion as with the
//! maps), so a pair with and without it shows the shadows alone.
//!
//! The lit shop windows' cards (`--shop-light`, stage N1): `EO_LIGHT_SHOTS`
//! naming `shops` adds `shops-castro-18th` (Castro Street from 18th, looking
//! north up the block toward Market from its south-east corner: the reference
//! photo's corner), `shops-castro-walk` (12 m up the block on its east walk)
//! and `shops-mission-24th` (Mission Street 12 m north of 24th on its east walk), at
//! night; with the option every card's JSON carries the frame's light census
//! (`shop_census_json`: the lights in the frame list by kind, the clusters
//! full and holding shops).
//!
//! The key views (the cinematic look plan, stage L0; `hero_views.rs`):
//! `EO_LIGHT_SHOTS=hero` draws the six `hero-*` cards, each under its own
//! weather and seed with the sky's clock held at `--world-time` and the
//! water's at a fixed time while it settles (a still frame that does not
//! depend on the settle's length).

use super::{Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::smooth_time;
use eo_render::{Renderer, Targets, wgpu};
use eo_sky::Kind;
use glam::Vec3;

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--light-shots")
}

/// Noon, golden hour (the sky cards' sun at about 8 degrees) and night.
const NOON: f64 = 12.0;
const LOW_SUN: f64 = 18.5;
const NIGHT: f64 = 21.0;
/// The dusk strip's hours: golden hour into the night.
const DUSK: [f64; 8] = [18.0, 18.5, 18.75, 19.0, 19.25, 19.5, 19.75, 20.25];
/// How far from the viaduct's centreline the under-viaduct eye may stand.
const VIADUCT_REACH_M: f32 = 60.0;
/// The headlight strip's car: moving faster than this (m/s; walkers are
/// under 2), within this distance of the junction card's eye (m), found
/// within this many frames of traffic.
const STRIP_CAR_MIN_MS: f32 = 5.0;
const STRIP_CAR_REACH_M: f32 = 80.0;
const STRIP_CAR_SEARCH_FRAMES: usize = 900;
/// Frames drawn at the strip's camera before the first written one (the
/// histories settle after the cut), and its distance from the car's path (m).
const STRIP_SETTLE_FRAMES: usize = 12;
const STRIP_SIDE_M: f32 = 11.0;
/// The strip's eye height (m: over the walkers' heads) and the radius round it in which no agent is drawn.
const STRIP_EYE_M: f32 = 2.4;
const STRIP_CLEAR_M: f32 = 5.0;

/// `EO_LIGHT_SHOTS_WEATHER`: every card under this preset instead of clear.
fn weather_override() -> Option<Kind> {
    std::env::var("EO_LIGHT_SHOTS_WEATHER").ok().and_then(|v| Kind::parse(v.trim()))
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

fn surface(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

fn on_surface(world: &World, p: Vec3, up: f32) -> Vec3 {
    Vec3::new(p.x, surface(world, p.x, p.z) + up, p.z)
}

/// The point `t` (0..1) of the way along a polyline.
fn along(points: &[Vec3], t: f32) -> Vec3 {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * t.clamp(0.0, 1.0);
    for w in points.windows(2) {
        let d = w[0].distance(w[1]);
        if left <= d && d > 0.0 {
            return w[0].lerp(w[1], left / d);
        }
        left -= d;
    }
    *points.last().unwrap_or(&Vec3::ZERO)
}

/// The longest street segment with this name (the loaded street city, on the scaled plan).
fn longest_segment(world: &World, name: &str) -> Option<Vec<Vec3>> {
    let sw = &world.street.as_ref()?.world;
    sw.city
        .segments
        .iter()
        .filter(|s| s.street_name == name && s.points.len() >= 2)
        .map(|s| s.points.iter().map(|p| Vec3::new(p[0] as f32, 0.0, p[1] as f32)).collect::<Vec<_>>())
        .max_by(|a, b| {
            let len = |p: &Vec<Vec3>| p.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
            len(a).total_cmp(&len(b))
        })
}

/// The plan junction where streets `a` and `b` meet (the first in plan order).
fn junction(world: &World, a: &str, b: &str) -> Option<Vec3> {
    let sw = &world.street.as_ref()?.world;
    let name = |ap: &eo_street::plan::Approach| sw.city.segments[sw.plan.entries[ap.entry].road].street_name.as_str();
    sw.plan.nodes.iter().find(|n| n.approaches.iter().any(|ap| name(ap) == a) && n.approaches.iter().any(|ap| name(ap) == b)).map(|n| Vec3::new(n.position[0] as f32, 0.0, n.position[1] as f32))
}

/// The nearest grade-separated motorway piece to `p` (the street city keeps
/// the ways the street surface skips): its distance and unit direction.
fn nearest_viaduct(world: &World, p: Vec3) -> Option<(f32, Vec3)> {
    let sw = &world.street.as_ref()?.world;
    let mut best: Option<(f32, Vec3)> = None;
    for s in sw.city.segments.iter().filter(|s| s.separated && s.highway.starts_with("motorway")) {
        for w in s.points.windows(2) {
            let (a, b) = (Vec3::new(w[0][0] as f32, 0.0, w[0][1] as f32), Vec3::new(w[1][0] as f32, 0.0, w[1][1] as f32));
            let ab = b - a;
            if ab.length_squared() < 1e-6 {
                continue;
            }
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let d = (a + ab * t).distance(Vec3::new(p.x, 0.0, p.z));
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, ab.normalize()));
            }
        }
    }
    best
}

/// The cards: (name, camera, hour, what), in capture order.
pub fn cards(world: &World, segs: &[Segment], args: &Args, aspect: f32) -> Vec<(String, Camera, f64, String)> {
    let far = args.far;
    let mut out = vec![];
    let missing = |what: &str| eprintln!("light-shots: {what} not found; its card is left out");
    // An alley: Maiden Lane, from 15 % along it looking to its far end.
    match longest_segment(world, "Maiden Lane") {
        Some(p) => {
            let (eye, at) = (on_surface(world, along(&p, 0.15), super::EYE_HEIGHT_M), on_surface(world, along(&p, 1.0), super::EYE_HEIGHT_M));
            let what = format!("Maiden Lane from ({:.0}, {:.0}), eye 1.7 m, looking along it", eye.x, eye.z);
            out.push(("alley".to_string(), look_at(eye, at, aspect, far), NOON, what.clone()));
            out.push(("alley-low-sun".to_string(), look_at(eye, at, aspect, far), LOW_SUN, what)); // the ambient volume's golden-hour pair
        }
        None => missing("Maiden Lane"),
    }
    // A plaza: the landmarks card `plaza` (Justin Herman Plaza from its south-east corner).
    match super::landmarks_bench::capture_cameras(world, aspect, far).into_iter().find(|(n, _, _)| *n == "plaza") {
        Some((_, cam, w)) => {
            out.push(("plaza".to_string(), cam, NOON, format!("the landmarks card plaza: {w}")));
            out.push(("plaza-low-sun".to_string(), cam, LOW_SUN, format!("the landmarks card plaza: {w}")));
        }
        None => missing("the landmarks card plaza"),
    }
    // The skyline and a hill's shadow at low sun: the far-city cards.
    let far_cards = super::far_shots::cameras(world, segs, args, aspect);
    for (card, name, what) in [("towers-roof", "skyline-low-sun", "the far-city card towers-roof"), ("twin-peaks-500m", "hill-shadow", "the far-city card twin-peaks-500m")] {
        match far_cards.iter().find(|(n, _, _)| n == card) {
            Some((_, cam, w)) => out.push((name.to_string(), *cam, LOW_SUN, format!("{what}: {w}"))),
            None => missing(card),
        }
    }
    // A parked row and a junction at night: the props cards.
    let props_cards = crate::props::capture_cameras(world, aspect, far);
    for (card, name) in [("parked", "parked-row-night"), ("junction", "junction-night")] {
        match props_cards.iter().find(|(n, _, _)| n == card) {
            Some((_, cam, w)) => out.push((name.to_string(), *cam, NIGHT, format!("the props card {card}: {w}"))),
            None => missing(card),
        }
    }
    // Under a viaduct: 5th Street between Harrison and Bryant, under I-80.
    match (junction(world, "5th Street", "Harrison Street"), junction(world, "5th Street", "Bryant Street")) {
        (Some(a), Some(b)) => {
            // the point of 5th Street (between the two junctions) nearest the viaduct's line.
            let on_fifth = (0..=40).filter_map(|i| {
                let p = a.lerp(b, i as f32 / 40.0);
                nearest_viaduct(world, p).map(|(d, dir)| (d, dir, p))
            });
            match on_fifth.min_by(|x, y| x.0.total_cmp(&y.0)).filter(|(d, _, _)| *d <= VIADUCT_REACH_M) {
                Some((d, dir, p)) => {
                    let eye = on_surface(world, p, super::EYE_HEIGHT_M);
                    let at = eye + dir * 60.0 + Vec3::Y * 3.0;
                    let what = format!("5th Street between Harrison and Bryant at ({:.0}, {:.0}), {d:.1} m from the I-80 viaduct's line (the nearest grade-separated motorway way), eye 1.7 m, looking along it", eye.x, eye.z);
                    out.push(("under-viaduct".to_string(), look_at(eye, at, aspect, far), NOON, what.clone()));
                    out.push(("under-viaduct-low-sun".to_string(), look_at(eye, at, aspect, far), LOW_SUN, what));
                }
                None => missing("a grade-separated motorway near 5th Street"),
            }
        }
        _ => missing("5th Street at Harrison and Bryant"),
    }
    // The far sun shadow maps' cards: the city from the air, the elevated sweep.
    for (path, frames) in [("city_overview", &[(0usize, "far-city-overview")][..]), ("elevated_sweep", &[(0, "far-elevated-sweep-0"), (1, "far-elevated-sweep-q")])] {
        match segs.iter().find(|s| s.name == path) {
            Some(s) => {
                let total = args.warmup + s.frames;
                for &(k, name) in frames {
                    let i = if k == 0 { 0 } else { total / 4 };
                    out.push((name.to_string(), (s.camera)(i, total), LOW_SUN, format!("the {path} path's frame {i} of {total}")));
                }
            }
            None => missing(&format!("the {path} path")),
        }
    }
    // The dusk strip: the kit street at eight hours.
    match segs.iter().find(|s| s.name == "kit_street") {
        Some(s) => {
            let cam = (s.camera)(260, args.warmup + s.frames);
            for h in DUSK {
                let name = format!("dusk-{:02}{:02}", h.trunc() as u32, (h.fract() * 60.0).round() as u32);
                out.push((name, cam, h, format!("the kit street (the sky cards' frame 260) at {h} h")));
            }
        }
        None => missing("the kit_street path"),
    }
    // (lighting, the lit fog's cards) the night kit street: the night_kit_street bench path's frames
    // (kit_street's cameras) at night; listed only when EO_LIGHT_SHOTS names them.
    if std::env::var("EO_LIGHT_SHOTS").is_ok_and(|v| v.contains("night-kit")) {
        match segs.iter().find(|s| s.name == "kit_street") {
            Some(s) => {
                let total = args.warmup + s.frames;
                for (name, i) in [("night-kit-street-0", 0), ("night-kit-street-mid", total / 2), ("night-kit-street-260", 260)] {
                    out.push((name.to_string(), (s.camera)(i, total), NIGHT, format!("the night_kit_street path's frame {i} of {total} (kit_street's camera)")));
                }
            }
            None => missing("the kit_street path"),
        }
    }
    // (`--shop-light`, stage N1) commercial blocks at night, when EO_LIGHT_SHOTS names `shops`:
    // the eye on the right-hand sidewalk (on the 1.25 ground plan: Castro's ~3.7 m walks 9.4-14 m
    // from its centre line, Mission's ~4.6 m walks 10-15.7 m) short of the cross street, looking up the block along the shopfronts.
    if std::env::var("EO_LIGHT_SHOTS").is_ok_and(|v| v.contains("shops")) {
        // (the way up the block: from the cross street behind toward the one at the eye, carried on.)
        // (the corner: 6 m short of the cross street; the walk: 12 m into the block past it)
        for (name, street, from, behind, walk, into) in [
            ("shops-castro-18th", "Castro Street", "18th Street", "19th Street", 11.0f32, -6.0f32),
            ("shops-castro-walk", "Castro Street", "18th Street", "19th Street", 11.0, 12.0),
            ("shops-mission-24th", "Mission Street", "24th Street", "25th Street", 12.5, 12.0),
        ] {
            match (junction(world, street, from), junction(world, street, behind)) {
                (Some(a), Some(b)) => {
                    let to = "the next block";
                    let d = Vec3::new(a.x - b.x, 0.0, a.z - b.z).normalize_or_zero();
                    let right = Vec3::new(-d.z, 0.0, d.x);
                    let eye = on_surface(world, a + d * into + right * walk, super::EYE_HEIGHT_M);
                    let at = on_surface(world, a + d * (into + 70.0) + right * (walk - 2.0), 2.0);
                    let what = format!("{street} from {from} at ({:.0}, {:.0}), eye 1.7 m on the right-hand walk {walk} m from the centre line, looking up {to} along the shopfronts", eye.x, eye.z);
                    out.push((name.to_string(), look_at(eye, at, aspect, far), NIGHT, what));
                }
                _ => missing(&format!("{street} at {from} and {behind}")),
            }
        }
    }
    // Opt-in free cameras, for matching a reference photo.
    // `EO_LIGHT_SHOTS_JUNCTION="A|B;C|D"` prints where those streets meet.
    // `EO_LIGHT_SHOTS_CAM="name,eye_x,eye_z,at_x,at_z,at_up[,fov_y_deg[,pitch_deg]];..."` (world metres)
    // adds a card per entry: the eye 1.7 m over the surface, looking at `at` `at_up` m over it
    // (a given pitch replaces the one toward `at`; only the heading is then taken from it).
    if let Ok(v) = std::env::var("EO_LIGHT_SHOTS_JUNCTION") {
        for pair in v.split(';').filter_map(|p| p.split_once('|')) {
            match junction(world, pair.0.trim(), pair.1.trim()) {
                Some(p) => eprintln!("light-shots: junction {} / {} at ({:.1}, {:.1}), surface {:.1} m", pair.0.trim(), pair.1.trim(), p.x, p.z, surface(world, p.x, p.z)),
                None => missing(&format!("the junction {} / {}", pair.0.trim(), pair.1.trim())),
            }
        }
    }
    if let Ok(v) = std::env::var("EO_LIGHT_SHOTS_CAM") {
        for spec in v.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let f: Vec<&str> = spec.split(',').map(str::trim).collect();
            let n: Vec<f32> = f.iter().skip(1).filter_map(|x| x.parse().ok()).collect();
            if !(6..=8).contains(&f.len()) || n.len() + 1 != f.len() {
                eprintln!("light-shots: EO_LIGHT_SHOTS_CAM entry \"{spec}\" is not name,eye_x,eye_z,at_x,at_z,at_up[,fov_y_deg[,pitch_deg]]; left out");
                continue;
            }
            let eye = on_surface(world, Vec3::new(n[0], 0.0, n[1]), super::EYE_HEIGHT_M);
            let at = on_surface(world, Vec3::new(n[2], 0.0, n[3]), n[4]);
            let mut cam = look_at(eye, at, aspect, far);
            if let Some(&fov) = n.get(5) {
                cam.fov_y_deg = fov;
            }
            if let Some(&pitch) = n.get(6) {
                cam.pitch = pitch.to_radians();
            }
            let what = format!("free camera: eye ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}), fov_y {:.1}", eye.x, eye.y, eye.z, at.x, at.y, at.z, cam.fov_y_deg);
            out.push((f[0].to_string(), cam, NOON, what));
        }
    }
    // (the cinematic look plan, stage L0) the six key views, when EO_LIGHT_SHOTS names `hero`.
    if super::hero_views::requested() {
        out.extend(super::hero_views::cards(world, segs, aspect, far));
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    segs: &[Segment],
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Ok(spec) = std::env::var("EO_PROBE_STRIP") {
        return run_probe_strip(ctx, renderer, world, models, props, agents, segs, args, color, attach, resolve, targets, dir, label, &spec);
    }
    if let Ok(spec) = std::env::var("EO_HEADLIGHT_STRIP") {
        return run_headlight_strip(ctx, renderer, world, models, props, agents, args, color, attach, resolve, targets, dir, label, &spec);
    }
    let base = args.look.sky;
    let weather = weather_override().unwrap_or(Kind::Clear);
    // `--no-sky`: only the hour moves (as the sky cards).
    let legacy = base == SkyOptions::off();
    let aspect = color.width() as f32 / color.height() as f32;
    let only = std::env::var("EO_LIGHT_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k.trim())));
    // `EO_LIGHT_SHOTS_HOUR=H`: every card at hour H.
    let hour_override = std::env::var("EO_LIGHT_SHOTS_HOUR").ok().and_then(|v| v.parse::<f64>().ok());
    let mut written = vec![];
    // (lighting) `EO_LIGHT_SHADOW_MAPS_OFF=1`: the light shadows' maps drawn but read by no light (the pair shows the shadows alone).
    renderer.light_shadow.set_maps_read(std::env::var_os("EO_LIGHT_SHADOW_MAPS_OFF").is_none());
    for (name, cam, hour, what) in cards(world, segs, args, aspect) {
        if !want(&name) {
            continue;
        }
        let hour = hour_override.unwrap_or(hour);
        let t = std::time::Instant::now();
        eprintln!("light-shots: {name}: settling at eye ({:.0}, {:.0}, {:.0}), {hour} h", cam.eye.x, cam.eye.y, cam.eye.z);
        renderer.look.set_time_of_day(hour);
        // (stage L0) a key view: its own weather and seed, the sky's clock held and the water's fixed while it settles.
        let hero = name.starts_with("hero-").then(|| super::hero_views::find(&name)).flatten();
        if let Some(h) = hero {
            renderer.look.set_sky_options(if legacy { SkyOptions::off() } else { super::hero_views::sky(h, base) });
            renderer.look.sky.set_clock(base.world_time_s, 0.0);
            if let Some(w) = renderer.water.get() {
                w.set_time(super::hero_views::WATER_TIME_S);
                w.set_always(true);
            }
        } else {
            renderer.look.set_sky_options(if legacy { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(weather), ..base } });
        }
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        let p = dir.join(format!("capture-light-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        let weather_name = match hero {
            _ if legacy => "none (--no-sky)".to_string(),
            Some(h) if !h.rain_falling => format!("{} (wet ground, no rain falling)", h.weather.name()),
            Some(h) => h.weather.name().to_string(),
            None => weather.name().to_string(),
        };
        println!("wrote {} (eye {:.1} {:.1} {:.1}, {hour} h, {weather_name}; {:.1} s)", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, t.elapsed().as_secs_f64());
        let mut card = serde_json::json!({ "name": name, "file": p.file_name().map(|f| f.to_string_lossy().to_string()), "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "hour": hour, "weather": weather_name, "what": what });
        if hero.is_some() {
            card["fov_y_deg"] = serde_json::json!(cam.fov_y_deg);
            card["far"] = serde_json::json!(cam.far);
            card["seed"] = serde_json::json!(super::hero_views::SEED);
            card["world_time_s"] = serde_json::json!(base.world_time_s);
        }
        if args.look.sky.shop_light > 0 {
            card["shop_lights"] = shop_census_json(ctx, renderer); // (`--shop-light`) only with the option
        }
        if args.look.lighting.light_shadows > 0 || args.look.lighting.headlight_shadows > 0 {
            card["light_shadows"] = light_shadows_json(ctx, renderer); // (lighting) only with the options
            if std::env::var_os("EO_LIGHT_SHADOW_ATLAS").is_some() {
                write_atlas(ctx, renderer, &dir.join(format!("capture-light-{label}-{name}-atlas.png")))?;
            }
        }
        written.push(card);
    }
    let manifest = dir.join(format!("capture-light-{label}.json"));
    let mut m = serde_json::json!({ "label": label, "cards": written, "structures": crate::structures::json(args, world), "lighting": crate::quality::lighting_json(args), "look_profile": crate::quality::look_profile_json(args), "quality": args.quality.name() });
    if args.look.lighting.lit_fog > 0 {
        m["lit_fog"] = lit_fog_json(renderer); // (lighting) only with --lit-fog
    }
    m["rt_gi"] = crate::quality::rt_gi_json(args, renderer); // (`--rt-gi`) whether the pass ran, its settings and what it traced
    if args.look.sky.shop_light > 0 {
        m["shop_light"] = crate::quality::shop_light_json(args, eo_render::shop_lights::census().as_ref()); // (`--shop-light`)
    }
    std::fs::write(&manifest, serde_json::to_vec_pretty(&m)?)?;
    println!("wrote {}", manifest.display());
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    if super::hero_views::requested() {
        renderer.look.sky.set_clock(renderer.look.sky.world_time(), 1.0 / 60.0); // (stage L0) the clock moves again
        if let Some(w) = renderer.water.get() {
            w.set_always(false);
        }
    }
    Ok(())
}

/// `--dusk-strip` (see the module's head).
pub fn strip_requested() -> bool {
    std::env::args().any(|a| a == "--dusk-strip")
}

/// Frames of a strip, and the clock between two.
const STRIP_FRAMES: usize = 40;
const STRIP_STEP_H: f64 = 1.0 / 60.0;
/// Frames rendered on the way from one strip frame's hour to the next's (the
/// soft path steps the clock through them; the hard path holds the new hour),
/// before the capture's own 20.
const STRIP_SUBSTEPS: usize = 10;

/// Mean luminance of an sRGB capture: (whole frame, top third, bottom two
/// thirds) of linear relative luminance, and the whole frame's mean display luma (0..1).
fn png_means(path: &std::path::Path) -> Result<(f64, f64, f64, f64)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut r = dec.read_info()?;
    let mut buf = vec![0u8; r.output_buffer_size().ok_or_else(|| anyhow::anyhow!("png size"))?];
    let info = r.next_frame(&mut buf)?;
    let (w, h, c) = (info.width as usize, info.height as usize, info.color_type.samples());
    let lin = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (mut all, mut top, mut bottom, mut disp) = (0.0, 0.0, 0.0, 0.0);
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) * c;
            let l = 0.2126 * lin(buf[p]) + 0.7152 * lin(buf[p + 1]) + 0.0722 * lin(buf[p + 2]);
            all += l;
            if y < h / 3 { top += l } else { bottom += l }
            disp += (0.2126 * buf[p] as f64 + 0.7152 * buf[p + 1] as f64 + 0.0722 * buf[p + 2] as f64) / 255.0;
        }
    }
    let n = (w * h) as f64;
    Ok((all / n, top / (w * (h / 3)).max(1) as f64, bottom / (w * (h - h / 3)).max(1) as f64, disp / n))
}

/// One frame (the capture's own loop, without the read back).
fn draw(ctx: &eo_render::GpuContext, renderer: &Renderer, attach: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, targets: &Targets, cam: &Camera) {
    renderer.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    renderer.encode(&mut enc, attach, resolve, &targets.depth, None);
    eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
}

#[allow(clippy::too_many_arguments)]
pub fn run_strip(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    segs: &[Segment],
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let base = args.look.sky;
    let legacy = base == SkyOptions::off();
    let smooth = args.look.lighting.smooth_time;
    let Some(seg) = segs.iter().find(|s| s.name == "kit_street") else {
        anyhow::bail!("dusk strip: the kit_street path is not loaded");
    };
    let cam = (seg.camera)(260, args.warmup + seg.frames);
    let only = std::env::var("EO_DUSK_STRIP").ok();
    let strips = [("dusk", smooth_time::horizon_hour(16.0, 23.0, false)), ("dawn", smooth_time::horizon_hour(3.0, 11.0, true))];
    let mut out = vec![];
    for (strip, horizon) in strips {
        if only.as_deref().is_some_and(|o| o != strip) {
            continue;
        }
        let first = horizon - (STRIP_FRAMES / 2) as f64 * STRIP_STEP_H;
        eprintln!("dusk strip: {strip} ({} path), the sun on the horizon at {horizon:.4} h, frames {first:.4} .. {:.4} h", if smooth { "soft" } else { "hard" }, first + (STRIP_FRAMES - 1) as f64 * STRIP_STEP_H);
        // the minute before the first frame, settled: the same start on both runs.
        let mut prev = first - STRIP_STEP_H;
        renderer.look.set_time_of_day(prev);
        renderer.look.set_sky_options(if legacy { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(Kind::Clear), ..base } });
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        // pin the world clock (the clouds' drift) and the frame time: from here both runs render the same frames.
        renderer.look.sky.set_clock(base.world_time_s, 1.0 / 60.0);
        for _ in 0..20 {
            draw(ctx, renderer, attach, resolve, targets, &cam);
        }
        let mut rows: Vec<serde_json::Value> = vec![];
        let mut csv = String::from("frame,hour,sun_altitude_deg,mean_luminance,delta_luminance,delta_stops,mean_display,delta_display_255,sky_luminance,ground_luminance,exposure,keys_built,keys_from_worker,probe_captures\n");
        let mut last: Option<(f64, f64)> = None;
        let (mut worst_stops, mut worst_disp) = ((0.0f64, 0usize), (0.0f64, 0usize));
        for n in 0..STRIP_FRAMES {
            let hour = first + n as f64 * STRIP_STEP_H;
            if smooth {
                for k in 1..=STRIP_SUBSTEPS {
                    renderer.look.set_time_of_day_smooth(prev + (hour - prev) * k as f64 / STRIP_SUBSTEPS as f64);
                    draw(ctx, renderer, attach, resolve, targets, &cam);
                }
            } else {
                renderer.look.set_time_of_day(hour);
                for _ in 0..STRIP_SUBSTEPS {
                    draw(ctx, renderer, attach, resolve, targets, &cam);
                }
            }
            let p = dir.join(format!("capture-light-{label}-{strip}-{n:02}.png"));
            capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
            let (mean, sky, ground, disp) = png_means(&p)?;
            let (d, stops, dd) = match last {
                Some((m0, d0)) => (mean - m0, (mean.max(1e-9) / m0.max(1e-9)).log2(), (disp - d0) * 255.0),
                None => (0.0, 0.0, 0.0),
            };
            if stops.abs() > worst_stops.0.abs() {
                worst_stops = (stops, n);
            }
            if dd.abs() > worst_disp.0.abs() {
                worst_disp = (dd, n);
            }
            last = Some((mean, disp));
            let alt = smooth_time::sun_altitude_deg(hour);
            let exposure = renderer.look.sky.drawn_exposure().unwrap_or(f64::NAN);
            let (built, prefetched) = renderer.look.smooth_time_keys();
            let probe = renderer.probe.stats().captures;
            csv.push_str(&format!("{n},{hour:.6},{alt:.4},{mean:.6},{d:.6},{stops:.5},{disp:.6},{dd:.4},{sky:.6},{ground:.6},{exposure:.5},{built},{prefetched},{probe}\n"));
            println!("wrote {} ({hour:.4} h, sun {alt:.2} deg, mean luminance {mean:.5}, step {stops:+.4} stops / {dd:+.2} display levels)", p.display());
            rows.push(serde_json::json!({ "frame": n, "file": p.file_name().map(|f| f.to_string_lossy().to_string()), "hour": hour, "sun_altitude_deg": alt, "mean_luminance": mean, "delta_stops": stops, "delta_display_255": dd, "exposure": exposure }));
            prev = hour;
        }
        let csv_path = dir.join(format!("capture-light-{label}-{strip}.csv"));
        std::fs::write(&csv_path, csv)?;
        println!("wrote {}: largest step {:+.4} stops (into frame {}), {:+.2} display levels (into frame {})", csv_path.display(), worst_stops.0, worst_stops.1, worst_disp.0, worst_disp.1);
        out.push(serde_json::json!({ "strip": strip, "horizon_hour": horizon, "first_hour": first, "step_h": STRIP_STEP_H, "substeps": STRIP_SUBSTEPS, "csv": csv_path.file_name().map(|f| f.to_string_lossy().to_string()), "largest_step_stops": worst_stops.0, "largest_step_stops_frame": worst_stops.1, "largest_step_display_255": worst_disp.0, "largest_step_display_frame": worst_disp.1, "frames": rows }));
    }
    let manifest = dir.join(format!("capture-light-{label}-strips.json"));
    let eye = serde_json::json!([cam.eye.x, cam.eye.y, cam.eye.z]);
    std::fs::write(&manifest, serde_json::to_vec_pretty(&serde_json::json!({ "label": label, "path": if smooth { "soft (--smooth-time)" } else { "hard" }, "camera": { "what": "the kit street (the sky cards' frame 260)", "eye": eye, "yaw": cam.yaw, "pitch": cam.pitch }, "weather": if legacy { "none (--no-sky)" } else { "clear" }, "strips": out, "lighting": crate::quality::lighting_json(args) }))?)?;
    println!("wrote {}", manifest.display());
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    Ok(())
}

/// The probe grid's numbers for the bench JSON (`--probe-grid`, only when attached).
pub fn probe_grid_json(renderer: &Renderer) -> serde_json::Value {
    let g = &renderer.probe_grid;
    let s = g.stats();
    let lv = g.level();
    serde_json::json!({
        "probes": lv.map(|l| l.probes), "slots": lv.map(|l| l.slots), "face_px": lv.map(|l| l.face), "levels": lv.map(|l| l.mips),
        "bytes": g.bytes(), "anchors": g.anchors(), "anchor_spacing_m": eo_render::probe_grid::ANCHOR_SPACING_M, "fade_s": eo_render::probe_grid::FADE_S,
        "captures": s.captures, "recaptures": s.recaptures, "faces": s.faces, "deferred": s.deferred, "restarted": s.restarted, "abandoned": s.abandoned,
        "frames_per_capture_last": s.last_capture_frames,
        "frames_per_capture_mean": if s.captures > 0 { s.capture_frames_sum as f64 / s.captures as f64 } else { 0.0 },
    })
}

/// (lighting, `--light-shadows`, `--headlight-shadows`) The light shadows'
/// atlas and this frame's selection, for a card's JSON.
pub fn light_shadows_json(ctx: &eo_render::GpuContext, renderer: &Renderer) -> serde_json::Value {
    let mut out = light_shadows_stats_json(renderer);
    out["selection"] = serde_json::json!(renderer.light_shadow.readback(ctx).map(|s| serde_json::json!({
            "lamps_selected": s.selected, "lamp_maps_drawn_this_frame": s.lamp_redraws, "headlight_maps_drawn_this_frame": s.head_redraws,
            "new_lamp_maps_this_frame": s.new_maps, "new_lamp_maps_since_start": s.new_total, "frames": s.frames,
            "vehicle_candidates": s.candidates, "vehicles": s.vehicles,
            "frame_lamps_reading_a_map": s.lamps_shadowed, "frame_headlamps_reading_a_map": s.headlamps_shadowed,
            "lamps": s.lamps.iter().map(|(k, t)| serde_json::json!({ "head": k.map(f32::from_bits), "tile": if *t == u32::MAX { None } else { Some(*t) } })).collect::<Vec<_>>(),
            "caster_records_this_frame": s.lists.iter().map(|l| l.0).collect::<Vec<_>>(),
            "caster_triangles_this_frame": s.lists.iter().map(|l| l.1).collect::<Vec<_>>(),
        })));
    out
}

/// (lighting) The light shadows' atlas and what they did over a run (the bench JSON; no GPU wait).
pub fn light_shadows_stats_json(renderer: &Renderer) -> serde_json::Value {
    let st = renderer.light_shadow.stats();
    serde_json::json!({
        "layout": st.layout.map(|l| serde_json::json!({ "lamps": l.lamps, "moving": l.moving, "headlights": l.heads, "columns": l.cols, "rows": l.rows, "atlas_px": l.atlas_px() })),
        "bytes": st.bytes, "frames": st.frames, "skipped": st.skipped,
        "new_maps_per_frame_max": eo_render::light_shadow::NEW_MAPS_PER_FRAME,
    })
}

/// `EO_LIGHT_SHADOW_ATLAS=1`: the light shadow atlas as a grey PNG (near black, far white).
fn write_atlas(ctx: &eo_render::GpuContext, renderer: &Renderer, path: &std::path::Path) -> Result<()> {
    let Some((w, h, d)) = renderer.light_shadow.atlas_readback(ctx) else { return Ok(()) };
    let rgba: Vec<u8> = d.iter().flat_map(|v| {
        let g = (*v >> 8) as u8;
        [g, g, g, 255]
    }).collect();
    write_png(path, w, h, &rgba)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// One frame drawn and read back (RGBA8 rows, tightly packed).
fn draw_read(ctx: &eo_render::GpuContext, renderer: &Renderer, color: &wgpu::Texture, attach: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, targets: &Targets, cam: &Camera) -> Result<Vec<u8>> {
    let (w, h) = (color.width(), color.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("probe-strip"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    renderer.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    renderer.encode(&mut enc, attach, resolve, &targets.depth, None);
    enc.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
    let bytes = eo_render::read_mapped(ctx, &buf)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + w as usize * 4]);
    }
    Ok(rgba)
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// Cells of the strip's regional measure.
const STRIP_CELLS: (usize, usize) = (16, 9);

/// Mean linear luminance of a frame and the mean display luma (0..255) of each cell.
fn frame_means(rgba: &[u8], w: usize, h: usize) -> (f64, Vec<f64>) {
    let lin = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (cx, cy) = STRIP_CELLS;
    let mut cells = vec![0.0; cx * cy];
    let mut all = 0.0;
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) * 4;
            let (r, g, b) = (rgba[p], rgba[p + 1], rgba[p + 2]);
            all += 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
            cells[(y * cy / h) * cx + x * cx / w] += 0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64;
        }
    }
    let per = (w * h) as f64 / (cx * cy) as f64;
    (all / (w * h) as f64, cells.into_iter().map(|c| c / per).collect())
}

/// `EO_PROBE_STRIP` (see the module's head).
#[allow(clippy::too_many_arguments)]
pub fn run_probe_strip(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    segs: &[Segment],
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
    spec: &str,
) -> Result<()> {
    let mut parts = spec.split(',').map(str::trim);
    let path = parts.next().filter(|p| !p.is_empty()).unwrap_or("kit_street");
    let frames: usize = parts.next().and_then(|v| v.parse().ok()).unwrap_or(120);
    let step: f32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let turn: f32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(-1.0);
    let Some(seg) = segs.iter().find(|s| s.name == path) else {
        anyhow::bail!("probe strip: the {path} path is not loaded");
    };
    // the path's line from 45 % on, finely sampled; the eye on the ground of the path's own camera.
    const N: usize = 1 << 16;
    let at = |t: f32| (seg.camera)(((t.clamp(0.0, 1.0)) * N as f32) as usize, N).eye;
    let (a, b) = (at(0.45), at(0.46));
    let along = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or(Vec3::X);
    let per_t = Vec3::new(b.x - a.x, 0.0, b.z - a.z).length() / 0.01; // metres of path per unit t
    let side = Vec3::new(-along.z, 0.0, along.x) * turn;
    let look = (along + side).normalize();
    let aspect = color.width() as f32 / color.height() as f32;
    let cam_at = |k: usize| {
        let eye = at(0.45 + k as f32 * step / per_t.max(1e-3));
        Camera { far: args.far, ..Camera::new(eye, yaw_of(look), 0.02, aspect) }
    };
    let base = args.look.sky;
    let hour = std::env::var("EO_LIGHT_SHOTS_HOUR").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(args.time_of_day);
    renderer.look.set_time_of_day(hour);
    renderer.look.set_sky_options(if base == SkyOptions::off() { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(Kind::Clear), ..base } });
    let first = cam_at(0);
    eprintln!("probe strip: {path} from ({:.0}, {:.0}), {frames} frames of {step} m, looking {:.0} deg across, {hour} h", first.eye.x, first.eye.z, if turn < 0.0 { -45.0 } else { 45.0 });
    // streamed round the strip's middle (the whole strip within half its length of it), nothing streamed during it: both runs draw the same geometry.
    settle_p(ctx, renderer, world, models, Some(&mut *props), &cam_at(frames / 2), 600.0)?;
    settle_p(ctx, renderer, world, models, Some(&mut *props), &first, 600.0)?;
    crate::props::sync_parked(ctx, renderer, props, agents, &first);
    // the same start on both runs: the clock pinned, every probe of this camera captured and faded in.
    renderer.look.sky.set_clock(base.world_time_s, 1.0 / 60.0);
    for k in 0..eo_render::probe_grid::SETTLE_FRAMES_MAX + 20 {
        draw(ctx, renderer, attach, resolve, targets, &first);
        if k >= 20 && !renderer.probe.capturing() && renderer.probe_grid.settled() {
            break;
        }
    }
    let (w, h) = (color.width() as usize, color.height() as usize);
    let fade = (eo_render::probe_grid::FADE_S * 60.0).round() as usize;
    let grid = renderer.probe_grid.active(&renderer.look);
    // EO_PROBE_STRIP_FREEZE=1: the probes held as captured at the first camera (a reference run:
    // a strip minus its reference cancels the camera's own motion, leaving what the probes changed).
    let frozen = std::env::var_os("EO_PROBE_STRIP_FREEZE").is_some();
    renderer.probe.set_frozen(frozen);
    renderer.probe_grid.set_frozen(frozen);
    let mut cells_csv = String::new();
    let mut csv = String::from("frame,eye_x,eye_z,single_captures,grid_captures,event,mean_luminance,delta_stops,largest_cell_change_255\n");
    let mut last: Option<(f64, Vec<f64>, Vec<u8>)> = None;
    let mut keep: Vec<usize> = vec![];
    let mut events: Vec<serde_json::Value> = vec![];
    let mut changes: Vec<(usize, f64, bool)> = vec![];
    let (mut single0, mut grid0) = (renderer.probe.stats().captures, renderer.probe_grid.stats().captures);
    for k in 0..frames {
        let cam = cam_at(k);
        let rgba = draw_read(ctx, renderer, color, attach, resolve, targets, &cam)?;
        let (single, gridc) = (renderer.probe.stats().captures, renderer.probe_grid.stats().captures);
        // a capture finished in this frame's encode: the single probe's cube is drawn from this frame,
        // the grid's from the next (fading in, or cross-fading from the old cube).
        let event = single > single0 || gridc > grid0;
        (single0, grid0) = (single, gridc);
        let (mean, cells) = frame_means(&rgba, w, h);
        let (stops, cell) = match &last {
            Some((m0, c0, _)) => ((mean.max(1e-9) / m0.max(1e-9)).log2(), cells.iter().zip(c0).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max)),
            None => (0.0, 0.0),
        };
        if k > 0 {
            changes.push((k, cell, event));
        }
        if event && events.len() < 8 {
            if let Some((_, _, prev)) = &last {
                let p = dir.join(format!("capture-light-{label}-probe-strip-f{:03}.png", k - 1));
                write_png(&p, w as u32, h as u32, prev)?;
            }
            for f in [k, k + 1, k + fade / 2, k + fade] {
                keep.push(f);
            }
            events.push(serde_json::json!({ "frame": k, "single_captures": single, "grid_captures": gridc, "largest_cell_change_255": cell }));
        }
        if keep.contains(&k) {
            let p = dir.join(format!("capture-light-{label}-probe-strip-f{k:03}.png"));
            write_png(&p, w as u32, h as u32, &rgba)?;
        }
        csv.push_str(&format!("{k},{:.2},{:.2},{single},{gridc},{},{mean:.6},{stops:.5},{cell:.3}\n", cam.eye.x, cam.eye.z, event as u8));
        cells_csv.push_str(&format!("{k},{}\n", cells.iter().map(|c| format!("{c:.3}")).collect::<Vec<_>>().join(",")));
        last = Some((mean, cells, rgba));
    }
    renderer.probe.set_frozen(false);
    renderer.probe_grid.set_frozen(false);
    let csv_path = dir.join(format!("capture-light-{label}-probe-strip.csv"));
    std::fs::write(&csv_path, csv)?;
    // every cell's mean display luma a frame (16 x 9, row by row), for a strip minus its reference.
    std::fs::write(dir.join(format!("capture-light-{label}-probe-strip-cells.csv")), cells_csv)?;
    // the regional change into an event frame against the frames with none (the camera's own motion).
    let mut quiet: Vec<f64> = changes.iter().filter(|c| !c.2).map(|c| c.1).collect();
    quiet.sort_by(f64::total_cmp);
    let pct = |v: &[f64], p: f64| if v.is_empty() { 0.0 } else { v[((v.len() - 1) as f64 * p).round() as usize] };
    let at_events: Vec<f64> = changes.iter().filter(|c| c.2).map(|c| c.1).collect();
    let summary = serde_json::json!({
        "label": label, "path": path, "frames": frames, "metres_a_frame": step, "turn": turn, "hour": hour,
        "probe_grid": grid, "probes_frozen": frozen, "first_eye": [first.eye.x, first.eye.y, first.eye.z], "yaw": first.yaw,
        "events": events,
        "largest_cell_change_at_events_255": at_events,
        "largest_cell_change_without_event_255": { "median": pct(&quiet, 0.5), "p95": pct(&quiet, 0.95), "max": quiet.last().copied().unwrap_or(0.0) },
        "csv": csv_path.file_name().map(|f| f.to_string_lossy().to_string()),
        "lighting": crate::quality::lighting_json(args),
        "probe_grid_stats": renderer.probe_grid.attached().then(|| probe_grid_json(renderer)),
    });
    let manifest = dir.join(format!("capture-light-{label}-probe-strip.json"));
    std::fs::write(&manifest, serde_json::to_vec_pretty(&summary)?)?;
    println!("wrote {} and {}: {} finished captures; largest cell change into them {:?}, without one median {:.2} p95 {:.2} max {:.2} (display levels)", csv_path.display(), manifest.display(), at_events.len(), at_events, pct(&quiet, 0.5), pct(&quiet, 0.95), quiet.last().copied().unwrap_or(0.0));
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    Ok(())
}

/// `EO_HEADLIGHT_STRIP` (see the module's head): a car's headlamps crossing
/// a fixed view, every frame written, for the lit fog's beams (`--lit-fog`:
/// they must move with the car and leave no trail).
#[allow(clippy::too_many_arguments)]
pub fn run_headlight_strip(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
    spec: &str,
) -> Result<()> {
    let frames: usize = spec.trim().parse().ok().filter(|n| *n > 0).unwrap_or(60);
    let base = args.look.sky;
    let weather = weather_override().unwrap_or(Kind::Fog);
    let hour = std::env::var("EO_LIGHT_SHOTS_HOUR").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(NIGHT);
    let aspect = color.width() as f32 / color.height() as f32;
    let dt = 1.0 / 60.0;
    // a street with traffic: the props card `junction`.
    let Some((_, start, _)) = crate::props::capture_cameras(world, aspect, args.far).into_iter().find(|(n, _, _)| n == "junction") else {
        anyhow::bail!("headlight strip: the props card junction is not found");
    };
    renderer.look.set_time_of_day(hour);
    renderer.look.set_sky_options(if base == SkyOptions::off() { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(weather), ..base } });
    settle_p(ctx, renderer, world, models, Some(&mut *props), &start, 600.0)?;
    crate::props::sync_parked(ctx, renderer, props, agents, &start);
    let focus = crate::agents::AgentRuntime::focus_of(start.eye, start.forward());
    // the traffic runs until a car passes near the card's eye.
    let mut pick = None;
    for _ in 0..STRIP_CAR_SEARCH_FRAMES {
        agents.pump(ctx, renderer, focus, dt);
        draw(ctx, renderer, attach, resolve, targets, &start);
        pick = agents
            .snapshot()
            .into_iter()
            .filter(|a| a.speed > STRIP_CAR_MIN_MS && Vec3::from(a.pos).distance(start.eye) < STRIP_CAR_REACH_M)
            .min_by(|a, b| Vec3::from(a.pos).distance(start.eye).total_cmp(&Vec3::from(b.pos).distance(start.eye)));
        if pick.is_some() {
            break;
        }
    }
    let Some(car) = pick else {
        anyhow::bail!("headlight strip: no car over {STRIP_CAR_MIN_MS} m/s within {STRIP_CAR_REACH_M} m of the junction card in {STRIP_CAR_SEARCH_FRAMES} frames");
    };
    let heading = {
        let h = glam::Quat::from_array(car.rot) * Vec3::Z;
        Vec3::new(h.x, 0.0, h.z).normalize_or(Vec3::X)
    };
    let side = Vec3::new(-heading.z, 0.0, heading.x);
    // where the car will be half way through the written frames (after the settle frames), on its heading.
    let mid = Vec3::from(car.pos) + heading * car.speed * (STRIP_SETTLE_FRAMES as f32 + frames as f32 * 0.5) * dt as f32;
    // the side of its path whose eye stands on the street surface with the clearer view (no wall in front of it).
    let cam_from = |s: f32| {
        let eye = on_surface(world, mid + side * s * STRIP_SIDE_M, STRIP_EYE_M);
        look_at(eye, mid + heading * 3.0 + Vec3::Y * 0.8, aspect, args.far)
    };
    let paved = |c: &Camera| world.street.as_ref().is_none_or(|st| st.world.paved_at(c.eye.x as f64, c.eye.z as f64).is_some());
    let cam = [1.0, -1.0].map(cam_from).into_iter().find(|c| paved(c) && super::blocked_view(c, super::CAPTURE_WALL_WARN_M).is_none()).unwrap_or_else(|| cam_from(1.0));
    // (lighting, the headlight shadows' evidence) `EO_HEADLIGHT_STRIP_VIEW=behind`: from behind the
    // car where it starts, 3 m up, looking along its heading (its beams and what they throw shadows of).
    let cam = if std::env::var("EO_HEADLIGHT_STRIP_VIEW").is_ok_and(|v| v == "behind") {
        let start = Vec3::from(car.pos);
        look_at(on_surface(world, start - heading * 9.0, 3.0), start + heading * 25.0, aspect, args.far)
    } else {
        cam
    };
    // no walker in front of the lens (agents within STRIP_CLEAR_M of the eye are not drawn).
    let hidden = std::mem::replace(&mut agents.hide_near, vec![(cam.eye.to_array(), STRIP_CLEAR_M)]);
    eprintln!(
        "headlight strip: a car at ({:.0}, {:.0}) at {:.1} m/s heading ({:.2}, {:.2}); the camera at ({:.1}, {:.1}, {:.1}), {frames} frames, {hour} h, {}",
        car.pos[0], car.pos[2], car.speed, heading.x, heading.z, cam.eye.x, cam.eye.y, cam.eye.z, weather.name()
    );
    for _ in 0..STRIP_SETTLE_FRAMES {
        agents.pump(ctx, renderer, focus, dt);
        draw(ctx, renderer, attach, resolve, targets, &cam);
    }
    let (w, h) = (color.width() as usize, color.height() as usize);
    let mut rows = vec![];
    for k in 0..frames {
        agents.pump(ctx, renderer, focus, dt);
        let rgba = draw_read(ctx, renderer, color, attach, resolve, targets, &cam)?;
        let (mean, _) = frame_means(&rgba, w, h);
        let p = dir.join(format!("capture-light-{label}-headlights-f{k:02}.png"));
        write_png(&p, w as u32, h as u32, &rgba)?;
        rows.push(serde_json::json!({ "frame": k, "file": p.file_name().map(|f| f.to_string_lossy().to_string()), "mean_luminance": mean }));
    }
    agents.hide_near = hidden;
    println!("wrote {frames} frames capture-light-{label}-headlights-fNN.png in {}", dir.display());
    let manifest = dir.join(format!("capture-light-{label}-headlights.json"));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&serde_json::json!({
            "label": label, "hour": hour, "weather": weather.name(), "frames": rows, "frame_dt_s": dt,
            "car": { "pos": car.pos, "speed_m_s": car.speed, "heading": [heading.x, heading.z] },
            "camera": { "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "side_m": STRIP_SIDE_M },
            "lighting": crate::quality::lighting_json(args),
            "lit_fog": lit_fog_json(renderer),
        }))?,
    )?;
    println!("wrote {}", manifest.display());
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    Ok(())
}

/// The lit fog's numbers (`--lit-fog`): its level, froxels, bytes and frames.
/// (`--shop-light`) The last frame's lights: the frame list by kind (0 street
/// lamps, 1 headlamps, 2 points, 3 props street lights, 4 shop windows) and
/// the clusters (how many hold a light, are full at 32, hold a shop light;
/// the most a cluster held), with the shop lights handed to the sky.
pub fn shop_census_json(ctx: &eo_render::GpuContext, renderer: &Renderer) -> serde_json::Value {
    let Ok((lights, clusters)) = renderer.look.sky.lights_readback(ctx) else { return serde_json::Value::Null };
    let mut kinds = [0usize; 5];
    for l in &lights {
        kinds[(l.kind as usize).min(4)] += 1;
    }
    let shop = eo_render::shop_lights::SHOP_LIGHT_KIND;
    let used = clusters.iter().filter(|c| !c.is_empty()).count();
    let full = clusters.iter().filter(|c| c.len() >= eo_render::sky::CLUSTER_CAP as usize).count();
    let with_shops = clusters.iter().filter(|c| c.iter().any(|&i| lights.get(i as usize).is_some_and(|l| l.kind == shop))).count();
    let most = clusters.iter().map(|c| c.len()).max().unwrap_or(0);
    serde_json::json!({
        "placed": renderer.look.sky.shop_light_count(),
        "frame_lights": lights.len(),
        "frame_cap": eo_render::sky::FRAME_LIGHT_CAP,
        "by_kind": { "street_lamps": kinds[0], "headlamps": kinds[1], "points": kinds[2], "props_lamps": kinds[3], "shop_windows": kinds[4] },
        "clusters": { "used": used, "full": full, "with_shop_lights": with_shops, "most": most },
    })
}

pub fn lit_fog_json(renderer: &Renderer) -> serde_json::Value {
    let s = renderer.lit_fog.stats();
    serde_json::json!({ "level": s.level, "froxels": s.dims, "bytes": s.bytes, "frames": s.frames, "skipped": s.skipped })
}
