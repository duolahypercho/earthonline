//! the night, rain and fog bench paths, and the sky slice's capture set
//! (`--bench --sky-shots --out DIR --label NAME`): kit_street frame 260 and
//! sidewalk_walk frame 195 at noon, golden hour, blue hour and night; the city
//! from above at golden hour and at night; rain on the busy street at the
//! spawn; the marine layer over the hills from above; a close view of moving
//! traffic with and without the temporal resolve.

use super::{Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{Renderer, Targets, wgpu};
use eo_sky::Kind;
use glam::Vec3;

/// The hours of the capture set: noon, golden hour (sun ~8 deg), blue hour
/// (sun ~-4 deg on September 22 in San Francisco) and night.
pub const HOURS: [(&str, f64); 4] = [("noon", 12.0), ("golden", 18.5), ("blue", 19.4), ("night", 21.0)];

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--sky-shots")
}

/// The same cameras as four standard paths, under the night, rain and fog
/// (and the water slice's promenade at night).
pub fn extra_segments(all: Vec<Segment>) -> Vec<Segment> {
    all.into_iter()
        .filter_map(|mut s| {
            s.name = match s.name {
                "kit_street" => "night_kit_street",
                "street_walk" => "rain_street_walk",
                "elevated_sweep" => "fog_elevated_sweep",
                "waterfront" => "night_waterfront",
                _ => return None,
            };
            s.description = format!("(sky) {} -- {}", look_of(s.name).map(|(h, w)| format!("{h} h, {w:?}")).unwrap_or_default(), s.description);
            Some(s)
        })
        .collect()
}

fn look_of(name: &str) -> Option<(f64, WeatherMode)> {
    Some(match name {
        "night_kit_street" => (21.0, WeatherMode::Preset(Kind::Clear)),
        "rain_street_walk" => (12.0, WeatherMode::Preset(Kind::Rain)),
        "fog_elevated_sweep" => (17.0, WeatherMode::Preset(Kind::Fog)),
        "night_waterfront" => (21.0, WeatherMode::Preset(Kind::Clear)),
        _ => return None,
    })
}

/// Put a sky path's hour and weather in force; returns what to restore.
pub fn enter(r: &Renderer, args: &Args, name: &str) -> Option<(f64, SkyOptions)> {
    let (h, w) = look_of(name)?;
    let saved = (r.look.time_of_day(), r.look.sky.options());
    r.look.set_time_of_day(h);
    // With `--no-sky` (the A/B baseline) only the hour moves: the renderer
    // before this slice had no weather.
    if args.look.sky != SkyOptions::off() {
        r.look.set_sky_options(SkyOptions { weather: w, ..args.look.sky });
    }
    Some(saved)
}

pub fn leave(r: &Renderer, saved: Option<(f64, SkyOptions)>) {
    if let Some((h, o)) = saved {
        r.look.set_time_of_day(h);
        r.look.set_sky_options(o);
    }
}

pub fn describe(r: &Renderer) -> serde_json::Value {
    let w = r.look.sky.weather();
    let t = r.look.sky_terms();
    serde_json::json!({
        "options": format!("{:?}", r.look.sky.options()),
        "weather": { "kind": w.kind.name(), "severity": w.severity, "rain_mm_h": w.rain_mm_h, "wetness": w.wetness, "puddle_fill": w.puddle_fill },
        "exposure_target": t.map(|t| t.exposure_target),
        "lamps": r.look.sky.lamp_count(),
        "waterside_cells": r.look.sky.waterside_count(),
        "sky_bytes": r.look.sky.bytes + r.look.sky.target_bytes(),
    })
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
    let base = args.look.sky;
    let seg = |n: &str| segs.iter().find(|s| s.name == n);
    let aspect = color.width() as f32 / color.height() as f32;
    // `--no-sky`: the renderer before this slice (no weather either), for the matched "before" set.
    let legacy = base == SkyOptions::off();
    let mut shot = |name: String, cam: Camera, hour: f64, weather: Kind, opts: SkyOptions, moving: bool| -> Result<()> {
        let t_shot = std::time::Instant::now(); // progress per shot
        eprintln!("sky-shots: {name}: settling at eye ({:.0}, {:.0}, {:.0})", cam.eye.x, cam.eye.y, cam.eye.z);
        renderer.look.set_time_of_day(hour);
        renderer.look.set_sky_options(if legacy { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(weather), ..opts } });
        let settle_ms = settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        eprintln!("sky-shots: {name}: settled in {settle_ms:.0} ms");
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        if moving {
            // Traffic moves while the frames accumulate (the temporal resolve's test).
            for _ in 0..40 {
                agents.pump(ctx, renderer, crate::agents::AgentRuntime::focus_of(cam.eye, cam.forward()), 1.0 / 60.0);
                renderer.update(&ctx.queue, &cam);
                let mut enc = ctx.device.create_command_encoder(&Default::default());
                renderer.encode(&mut enc, attach, resolve, &targets.depth, None);
                ctx.queue.submit([enc.finish()]);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
        let p = dir.join(format!("capture-sky-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        println!("wrote {} (eye {:.1} {:.1} {:.1}, {hour} h, {}; {:.1} s)", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, weather.name(), t_shot.elapsed().as_secs_f64()); // + the shot's time
        Ok(())
    };
    let only = std::env::var("EO_SKY_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    for (hname, h) in HOURS {
        if let Some(s) = seg("kit_street") {
            let n = format!("kit_street-f260-{hname}");
            if want(&n) {
                shot(n, (s.camera)(260, args.warmup + s.frames), h, Kind::Clear, base, false)?;
            }
        }
        if let Some(s) = seg("sidewalk_walk") {
            let n = format!("sidewalk_walk-f195-{hname}");
            if want(&n) {
                shot(n, (s.camera)(195, args.warmup + s.frames), h, Kind::Clear, base, false)?;
            }
        }
    }
    // The city from above, from over the western hills toward downtown.
    let from = super::on_plan(Vec3::new(-3600.0, 650.0, 3300.0));
    let to = super::on_plan(Vec3::new(1800.0, 0.0, -1200.0)) - from;
    let above = Camera { far: args.far.max(9000.0), ..Camera::new(from, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect) };
    for (n, h) in [("above-golden", 18.5), ("above-night", 21.0)] {
        if want(n) {
            shot(n.into(), above, h, Kind::Clear, base, false)?;
        }
    }
    // The marine layer over the hills, from above, late afternoon.
    let fog_from = super::on_plan(Vec3::new(-5200.0, 900.0, 4200.0));
    let fog_to = super::on_plan(Vec3::new(-800.0, 0.0, 600.0)) - fog_from;
    let fog_cam = Camera { far: args.far.max(9000.0), ..Camera::new(fog_from, yaw_of(fog_to), (fog_to.y / Vec3::new(fog_to.x, 0.0, fog_to.z).length()).atan(), aspect) };
    if want("fog-above") {
        shot("fog-above".into(), fog_cam, 17.0, Kind::Fog, base, false)?;
    }
    // Rain on the busy street at the spawn (Folsom Street), by day and by night.
    if let Some(s) = seg("street_walk") {
        let cam = (s.camera)(30, args.warmup + s.frames);
        for (n, h) in [("rain-street-day", 12.0), ("rain-street-night", 21.0)] {
            if want(n) {
                shot(n.into(), cam, h, Kind::Rain, base, true)?;
            }
        }
        // Moving traffic close up: the temporal resolve on and off.
        let down = Camera { pitch: -0.12, ..cam };
        for (n, taa) in [("taa-cars-on", true), ("taa-cars-off", false)] {
            if want(n) {
                shot(n.into(), down, 12.0, Kind::Clear, SkyOptions { taa, ..base }, true)?;
            }
        }
    }
    // The waterfront at night (the water slice's cameras): the Embarcadero
    // promenade looking out over the bay, the piers from the water 3 m up,
    // and downtown across the bay from 1 km out, the lamps by the water seen
    // in it.
    for (wn, cam, _) in super::water_bench::capture_cameras(world, aspect, args.far) {
        if !matches!(wn, "embarcadero" | "pier" | "skyline-reflection") {
            continue;
        }
        let n = format!("waterfront-{wn}-night");
        if want(&n) {
            shot(n, cam, 21.0, Kind::Clear, base, false)?;
        }
    }
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    Ok(())
}
