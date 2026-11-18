//! so: the sky and ocean review cards (`--bench --so-shots --out DIR --label
//! NAME`): one fixed list of cameras, hours and weathers looking at the sea and
//! the sky, captured the same way every time so that two builds can be
//! compared card for card (same camera, hour, weather, water clock, fresh
//! boot each side). Env `EO_SO_SHOTS=a,b` keeps only the cards whose name
//! contains one of the keys. A `so-cards-<label>.json` beside the PNGs lists
//! every card with what it shows. Nothing here says how a frame looks.

use super::{Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{Renderer, Targets, wgpu};
use eo_sky::Kind;
use glam::Vec3;

/// an opt-in bench path (`EO_SURF_BENCH=1`): Ocean Beach at the waterline, eye 1.7 m over
/// the drawn sand, 300 m north along the beach looking out at the surf with a +-40 degree sweep.
pub fn surf_segment(world: &World, frames: usize, aspect: f32, far: f32) -> Option<super::Segment> {
    if std::env::var("EO_SURF_BENCH").map(|v| v == "0").unwrap_or(true) {
        return None;
    }
    let w = world.water.as_ref()?;
    let (cam, _) = super::water_bench::capture_cameras(world, aspect, far).into_iter().find(|c| c.0 == "ocean-beach").map(|c| (c.1, c.2))?;
    let f = Vec3::new(cam.forward().x, 0.0, cam.forward().z).normalize();
    let mut p = cam.eye;
    for _ in 0..200 {
        if w.field.signed_at(p.x as f64, p.z as f64) > -1.0 {
            break;
        }
        p += f * 0.5;
    }
    let north = Vec3::new(-f.z, 0.0, f.x) * if f.x < 0.0 { 1.0 } else { -1.0 };
    let world_water = world.water.clone()?;
    let ground: Vec<f32> = (0..=64).map(|k| {
        let q = p + north * (300.0 * k as f32 / 64.0);
        (world_water.field.press_beach(q.x as f64, q.z as f64, world.elevation.ground(q.x as f64, q.z as f64)).max(eo_water::SEA_LEVEL_Y) + 1.7) as f32
    }).collect();
    Some(super::Segment {
        name: "surf_beach",
        description: format!("Ocean Beach at the waterline ({:.0}, {:.0}), eye 1.7 m over the drawn sand, 300 m north along the beach looking out at the surf with a +-40 degree sweep", p.x, p.z),
        camera: Box::new(move |i, n| {
            let t = i as f32 / n.max(1) as f32;
            let k = ((t * 64.0) as usize).min(64);
            let q = p + north * (300.0 * t);
            let yaw = yaw_of(f) + 40f32.to_radians() * (std::f32::consts::TAU * t).sin();
            Camera { far, ..Camera::new(Vec3::new(q.x, ground[k], q.z), yaw, -0.02, aspect) }
        }),
        frames,
        paced: false,
        streams: true,
    })
}

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--so-shots")
}

/// (suffix, hour, weather): the conditions a camera is captured under.
type Look = (&'static str, f64, Kind);

const NOON: Look = ("noon", 12.0, Kind::Clear);
const MORNING: Look = ("morning", 7.5, Kind::Clear);
const GOLDEN: Look = ("golden", 18.5, Kind::Clear);
const BLUE: Look = ("blue", 19.4, Kind::Clear);
const NIGHT: Look = ("night", 21.0, Kind::Clear);
const OVERCAST: Look = ("overcast", 12.0, Kind::Overcast);
const FOG: Look = ("fog", 17.0, Kind::Fog);
const RAIN: Look = ("rain", 12.0, Kind::Rain);
const STORM: Look = ("storm", 15.0, Kind::Storm);
const LATE: Look = ("late", 17.5, Kind::Clear);
const LOWSUN: Look = ("lowsun", 18.8, Kind::Clear);
const SUNSET: Look = ("sunset", 19.1, Kind::Clear);
const AFTERNOON: Look = ("afternoon", 16.0, Kind::Clear);

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
    let base = args.look.sky;
    let aspect = color.width() as f32 / color.height() as f32;
    // A still frame must not depend on the frames before it: the water clock is fixed and the FFT runs every frame.
    if let Some(w) = renderer.water.get() {
        w.set_time(20.0);
        w.set_always(true);
    }
    let water: Vec<(&'static str, Camera, String)> = super::water_bench::capture_cameras(world, aspect, args.far);
    let land: Vec<(&'static str, Camera, String)> = super::landmarks_bench::capture_cameras(world, aspect, args.far);
    let find = |list: &[(&'static str, Camera, String)], n: &str| list.iter().find(|c| c.0 == n).map(|c| (c.1, c.2.clone()));

    // (card, camera, what it shows, the looks it is captured under), grouped by place so a place settles once.
    let mut cards: Vec<(String, Camera, String, Vec<Look>)> = vec![];
    if let Some((cam, what)) = find(&water, "embarcadero") {
        cards.push(("ocean-promenade".into(), cam, what, vec![NOON, MORNING, GOLDEN]));
    }
    if let Some((cam, what)) = find(&water, "pier") {
        cards.push(("ocean-pier".into(), cam, what, vec![NOON, RAIN]));
        // Open water: the same spot 3 m over the bay, turned round to look away from the city.
        let open = Camera { yaw: cam.yaw + std::f32::consts::PI, pitch: -0.05, ..cam };
        cards.push(("ocean-open".into(), open, "open bay water from 3 m up, 260 m off the piers, looking away from the city".into(), vec![NOON, MORNING]));
    }
    if let Some((cam, what)) = find(&water, "skyline-reflection") {
        cards.push(("ocean-skyline".into(), cam, what, vec![NOON, GOLDEN, BLUE, NIGHT]));
    }
    if let Some((cam, what)) = find(&water, "bay-aerial") {
        cards.push(("ocean-bay-aerial".into(), cam, what, vec![NOON, GOLDEN]));
    }
    if let Some((cam, what)) = find(&land, "aquatic-park") {
        cards.push(("ocean-cove".into(), cam, what, vec![NOON]));
    }
    if let Some((cam, what)) = find(&land, "golden-gate") {
        cards.push(("ocean-golden-gate".into(), cam, what, vec![NOON, FOG]));
    }
    if let Some((cam, _)) = find(&water, "ocean-beach") {
        // The water slice's camera stands 40 m up the sand (the sea is a thin strip there): walk it
        // down to 6 m from the waterline so the surf and the sea fill the lower half of the frame.
        let f = Vec3::new(cam.forward().x, 0.0, cam.forward().z).normalize();
        let mut p = cam.eye;
        if let Some(w) = world.water.as_ref() {
            for _ in 0..80 {
                if w.field.signed_at(p.x as f64, p.z as f64) > -6.0 {
                    break;
                }
                p += f * 0.5;
            }
        }
        let y = (world.elevation.ground(p.x as f64, p.z as f64) as f32).max(eo_water::SEA_LEVEL_Y as f32) + 1.7;
        let cam = Camera { eye: Vec3::new(p.x, y, p.z), ..cam };
        let what = "Ocean Beach, on the sand 6 m from the waterline at eye height, looking out at the Pacific".to_string();
        cards.push(("ocean-beach".into(), cam, what.clone(), vec![NOON, GOLDEN, OVERCAST, STORM]));
        // The surf: the same spot, looking along the waterline.
        let along = Camera { yaw: cam.yaw + 1.05, pitch: -0.10, ..cam };
        cards.push(("ocean-surf".into(), along, "the surf at Ocean Beach, looking along the waterline from 6 m up the sand".into(), vec![NOON]));
        // The swash: at the surveyed waterline itself, looking straight out at the breakers.
        let mut q = p;
        if let Some(w) = world.water.as_ref() {
            for _ in 0..40 {
                if w.field.signed_at(q.x as f64, q.z as f64) > -1.0 {
                    break;
                }
                q += f * 0.5;
            }
        }
        let qy = (world.elevation.ground(q.x as f64, q.z as f64) as f32).max(eo_water::SEA_LEVEL_Y as f32) + 1.7;
        let swash = Camera { eye: Vec3::new(q.x, qy, q.z), pitch: -0.03, ..cam };
        cards.push(("ocean-swash".into(), swash, "Ocean Beach at the surveyed waterline (the swash zone) at eye height, looking straight out at the breakers".into(), vec![NOON]));
        // The surf from above: 18 m over the same sand, looking south-west along the surf, late afternoon.
        let high = Camera { far: args.far, ..Camera::new(cam.eye + Vec3::new(0.0, 18.0, 0.0), yaw_of(Vec3::new(-0.55, 0.0, 0.83)), -0.22, aspect) };
        cards.push(("ocean-surf-high".into(), high, "the surf at Ocean Beach from 18 m up, looking south-west along the waterline".into(), vec![LATE]));
        // The low sun over the Pacific with its disc in the frame (the view raised 5 degrees).
        let low = Camera { pitch: 0.09, ..cam };
        cards.push(("sky-pacific".into(), low, "the sun a few degrees over the Pacific horizon from Ocean Beach, the view raised 5 degrees".into(), vec![LOWSUN]));
        // The sky over the Pacific from the same spot: the view raised so the sky fills most of the frame.
        let up = Camera { pitch: 0.32, ..cam };
        cards.push(("sky-west".into(), up, format!("the sky over the Pacific (the view raised 18 degrees) from {what}"), vec![NOON, GOLDEN, BLUE, NIGHT, OVERCAST, STORM]));
        // Toward the noon sun (south), raised.
        let south = Camera { far: args.far, ..Camera::new(cam.eye, yaw_of(Vec3::new(0.0, 0.0, 1.0)), 0.30, aspect) };
        cards.push(("sky-south".into(), south, "the sky toward the noon sun from Ocean Beach, the view raised 17 degrees".into(), vec![NOON]));
        // Sunset: the low-sun camera with the sun touching the sea.
        cards.push(("sky-pacific-sunset".into(), low, "the sun touching the Pacific horizon from Ocean Beach, the view raised 5 degrees".into(), vec![SUNSET]));
        // Twilight looking away from the sunset: Ocean Beach looking east over the city, raised.
        let east = Camera { far: args.far, ..Camera::new(cam.eye, yaw_of(Vec3::new(1.0, 0.0, 0.0)), 0.20, aspect) };
        cards.push(("sky-east".into(), east, "the sky over the city looking east from Ocean Beach, the view raised 11 degrees".into(), vec![BLUE, GOLDEN]));
        // The open Pacific 600 m off Ocean Beach, 3 m up, looking out to sea: swell and wind sea away from the surf zone.
        let off = Vec3::new(cam.eye.x + f.x * 600.0, eo_water::SEA_LEVEL_Y as f32 + 3.0, cam.eye.z + f.z * 600.0);
        let pacific = Camera { far: args.far, ..Camera::new(off, cam.yaw, -0.04, aspect) };
        cards.push(("ocean-pacific".into(), pacific, "the open Pacific 600 m off Ocean Beach, 3 m over the water, looking out to sea".into(), vec![AFTERNOON]));
        // the breakers side-on and against the light (added with the surf strip; the cards above are unchanged).
        // At the surveyed waterline at eye height, looking north along the beach (the shore's tangent
        // from the shore field, turned 6 degrees toward the sea) so the breaker lines are seen edge-on.
        if let Some(w) = world.water.as_ref() {
            let g = |x: f32, z: f32| w.field.signed_at(x as f64, z as f64) as f32;
            let off = Vec3::new(g(q.x + 8.0, q.z) - g(q.x - 8.0, q.z), 0.0, g(q.x, q.z + 8.0) - g(q.x, q.z - 8.0)).normalize_or(Vec3::NEG_X);
            let mut north = Vec3::new(-off.z, 0.0, off.x);
            if north.z > 0.0 {
                north = -north;
            }
            let look = (north * 6f32.to_radians().cos() + off * 6f32.to_radians().sin()).normalize();
            // eye 1.7 m over the sand as drawn (the beach is laid on the foreshore under the surveyed ground)
            let sand = w.field.press_beach(q.x as f64, q.z as f64, world.elevation.ground(q.x as f64, q.z as f64)).max(eo_water::SEA_LEVEL_Y) as f32;
            let ey = sand + 1.7;
            let side = Camera { far: args.far, ..Camera::new(Vec3::new(q.x, ey, q.z), yaw_of(look), -0.01, aspect) };
            cards.push(("ocean-breakers-profile".into(), side, "Ocean Beach at the surveyed waterline at eye height, looking north along the beach: the breakers side-on".into(), vec![("1600", 16.0, Kind::Clear)]));
            // Backlit: the same eye turned toward the sun's bearing at 16:30.
            renderer.look.set_time_of_day(16.5);
            let k = renderer.look.frame().key_dir;
            let sun = Vec3::new(k[0] as f32, 0.0, k[2] as f32).normalize_or(off);
            let back = Camera { far: args.far, ..Camera::new(Vec3::new(q.x, ey, q.z), yaw_of(sun), 0.0, aspect) };
            cards.push(("ocean-breakers-backlit".into(), back, "Ocean Beach at the waterline at eye height, looking toward the sun: the breakers against the light".into(), vec![("1630", 16.5, Kind::Clear)]));
            renderer.look.set_time_of_day(args.time_of_day);
        }
    }
    // The Embarcadero seawall close up, where the water meets the wall. Looking down from the
    // promenade showed only paving, and walking the eye to the shore field's edge put it against the wall's face: the field's
    // edge, the ground and the drawn wall do not agree to within metres there. So the card is now
    // taken from the water: the eye walks out along the promenade view until 15 m of open water
    // (ground 0.3 m under the sea) lie behind it, stands 2 m over the sea and looks back at the
    // wall 40 degrees off square, so the wall face, its foot in the water and the water meet.
    if let Some((cam, _)) = find(&water, "embarcadero") {
        let sea = eo_water::SEA_LEVEL_Y as f32;
        let f = Vec3::new(cam.forward().x, 0.0, cam.forward().z).normalize();
        let mut p = cam.eye;
        let mut wet = 0.0f32;
        for _ in 0..600 {
            p += f * 0.5;
            if (world.elevation.ground(p.x as f64, p.z as f64) as f32) < sea - 0.3 {
                wet += 0.5;
                if wet >= 15.0 {
                    break;
                }
            } else {
                wet = 0.0;
            }
        }
        let side = Vec3::new(-f.z, 0.0, f.x);
        let back = (-f * 0.77 + side * 0.64).normalize();
        let off = Camera { far: args.far, ..Camera::new(Vec3::new(p.x, sea + 2.0, p.z), yaw_of(back), -0.10, aspect) };
        cards.push(("ocean-seawall".into(), off, "the Embarcadero seawall from the water, 2 m up and 15 m out, looking back at the wall where the bay meets it".into(), vec![NOON]));
    }
    // The central bay in the afternoon
    // westerly, 3 m over the water 1 km off the Ferry Building, looking north-west toward Alcatraz
    // and the Golden Gate (east is +x, north is -z): chop, whitecaps, gust patches and wind streaks.
    if let Some((cam, _)) = find(&water, "skyline-reflection") {
        let eye = Vec3::new(cam.eye.x, eo_water::SEA_LEVEL_Y as f32 + 3.0, cam.eye.z);
        let wind = Camera { far: args.far, ..Camera::new(eye, yaw_of(Vec3::new(-1.0, 0.0, -1.0)), -0.04, aspect) };
        cards.push(("ocean-bay-wind".into(), wind, "the central bay in the afternoon wind, 3 m over the water 1 km off the Ferry Building, looking north-west toward Alcatraz and the Golden Gate".into(), vec![AFTERNOON]));
    }
    // the far shore. The Embarcadero promenade's eye turned east-north-east across the bay
    // to the East Bay hills (Oakland, Berkeley), at golden hour and noon; and from 40 m over Aquatic
    // Park, north across the Golden Gate's mouth to Angel Island, Tiburon and Mount Tamalpais, late.
    if let Some((cam, _)) = find(&water, "embarcadero") {
        let east = Camera { far: args.far, ..Camera::new(cam.eye, yaw_of(Vec3::new(1.0, 0.0, -0.3)), 0.02, aspect) };
        cards.push(("farshore-eastbay".into(), east, "the East Bay hills across the bay from the Embarcadero promenade at eye height, looking east-north-east".into(), vec![GOLDEN, NOON]));
    }
    if let Some((cam, _)) = find(&land, "aquatic-park") {
        let north = Camera { far: args.far, ..Camera::new(cam.eye + Vec3::new(0.0, 40.0, 0.0), yaw_of(Vec3::new(-0.15, 0.0, -1.0)), 0.0, aspect) };
        cards.push(("farshore-marin".into(), north, "Angel Island, Tiburon and Mount Tamalpais from 40 m over Aquatic Park, looking north".into(), vec![LATE]));
    }
    // The city under the sky, from over the western hills toward downtown (the sky slice's camera).
    let from = super::on_plan(Vec3::new(-3600.0, 650.0, 3300.0));
    let to = super::on_plan(Vec3::new(1800.0, 0.0, -1200.0)) - from;
    let above = Camera { far: args.far.max(9000.0), ..Camera::new(from, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect) };
    cards.push(("sky-city-above".into(), above, "the city from 650 m over the western hills, looking toward downtown and the bay".into(), vec![NOON, GOLDEN, BLUE, NIGHT, FOG]));
    // The sky over a street (the kit street's frame 260).
    if let Some(s) = segs.iter().find(|s| s.name == "kit_street") {
        let cam = (s.camera)(260, args.warmup + s.frames);
        cards.push(("sky-street".into(), cam, "a residential street (Castro Street) at eye height".into(), vec![NOON, GOLDEN, BLUE, NIGHT]));
    }

    // the beach cards' water clock: each Ocean Beach card
    // is taken when the swash there has just drained back to the waterline (0.75 of its cycle,
    // the wet glaze behind it in the foreground), at the clock nearest 20 s; the other cards keep 20 s.
    let surf_strips = world.water.as_ref().map(|w| eo_water::surf_strip::build_strips(&w.field, &eo_water::load_beaches(&world.paths.repo_root).unwrap_or_default())).unwrap_or_default();
    let beach_card = |c: &str| ["ocean-beach", "ocean-surf", "ocean-swash", "ocean-breakers"].iter().any(|k| c.starts_with(k));
    let only = std::env::var("EO_SO_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    let mut manifest = vec![];
    for (card, cam, what, looks) in &cards {
        for (suffix, hour, weather) in looks {
            let name = format!("{card}-{suffix}");
            if !want(&name) {
                continue;
            }
            let t_shot = std::time::Instant::now();
            if let Some(w) = renderer.water.get() {
                // the beach cards' clock (above)
                let storm = if matches!(weather, Kind::Storm) { 1.0 } else { 0.0 };
                let clock = if beach_card(card) { eo_water::surf_strip::swash_clock(&surf_strips, cam.eye.x as f64, cam.eye.z as f64, 0.75, 20.0, storm).unwrap_or(20.0) } else { 20.0 };
                w.set_time(clock);
            }
            renderer.look.set_time_of_day(*hour);
            renderer.look.set_sky_options(SkyOptions { weather: WeatherMode::Preset(*weather), ..base });
            // The cloud clock is pinned and held still for every card: the settle runs a varying
            // number of updates (its time differs run to run), and with the clock running the same
            // card came out with different clouds in two runs of one build.
            renderer.look.sky.set_clock(base.world_time_s, 0.0);
            let settle_ms = settle_p(ctx, renderer, world, models, Some(&mut *props), cam, 600.0)?;
            crate::props::sync_parked(ctx, renderer, props, agents, cam);
            let p = dir.join(format!("so-{label}-{name}.png"));
            capture(ctx, renderer, color, attach, resolve, &targets.depth, cam, &p)?;
            println!(
                "wrote {} (eye {:.1} {:.1} {:.1}, {hour} h, {}; settle {settle_ms:.0} ms, {:.1} s)",
                p.display(),
                cam.eye.x,
                cam.eye.y,
                cam.eye.z,
                weather.name(),
                t_shot.elapsed().as_secs_f64()
            );
            manifest.push(serde_json::json!({
                "card": name,
                "file": p.file_name().map(|f| f.to_string_lossy().to_string()),
                "shows": what,
                "hour": hour,
                "weather": weather.name(),
                "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
                "yaw": cam.yaw,
                "pitch": cam.pitch,
            }));
        }
    }
    std::fs::write(dir.join(format!("so-cards-{label}.json")), serde_json::to_string_pretty(&manifest)?)?;
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    renderer.look.sky.set_clock(renderer.look.sky.world_time(), 1.0 / 60.0);
    Ok(())
}
