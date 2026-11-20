//! The facade clutter's capture card (stage W3): `--bench --clutter-shots
//! --out DIR --label NAME`, once with `--facade-clutter 1` (or 2) and once
//! without (one binary, a fresh boot each): the same cameras, hours and
//! settings. The bench paths kit_street (Castro Street's shops) and
//! authored_street at two frames each, and fronts the planner dresses at
//! level 1 (whatever the run draws, so both runs stand at the same places):
//! an older apartment block with fire escapes in the Tenderloin, Nob Hill
//! and Chinatown (from across the street, eye height), shop fronts with
//! awnings and signs in the Mission, the Castro and Chinatown, and a roof
//! from above; (stage S1) lettered signs close up and neon signs in shop
//! windows. `EO_CLUTTER_SHOTS=a,b` keeps the cards whose name contains
//! one of them; `EO_CLUTTER_SHOTS_HOUR=H` takes every card at hour H
//! (default 15). Files: `capture-clutter-<label>-<card>.png`.

use super::{EYE_HEIGHT_M, Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{Renderer, Targets, wgpu};
use glam::Vec3;

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--clutter-shots")
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

/// The district a footprint stands in (the planner's district/era prior; its cells are unscaled).
fn district_of(rec: &eo_kit::pipeline::LoadedRecord) -> Option<String> {
    let n = rec.polygon.len().max(1) as f64;
    let (x, z) = rec.polygon.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.z));
    let gs = eo_data::groundscale::scale();
    eo_kit::classify::prior_store()?.at(x / n / gs, z / n / gs)?.district
}

/// A dressed front: the first building in `district` (in record order) whose
/// level-1 clutter passes `good`, seen from across the street in front of
/// its first piece of one of `kinds`.
#[allow(clippy::too_many_arguments)]
fn front_view(
    world: &World,
    kc: &eo_kit::city::KitCity,
    district: &str,
    good: &dyn Fn(&eo_kit::pipeline::Planned, &eo_kit::clutter::Census) -> bool,
    kinds: &[&str],
    across: f32,
    look_up: f32,
    aspect: f32,
    far: f32,
) -> Option<(Camera, String)> {
    for (i, rec) in kc.records.iter().enumerate() {
        if kc.excluded.contains(&i) || district_of(rec).as_deref() != Some(district) {
            continue;
        }
        if world.kit.as_ref().is_some_and(|k| k.building_group.get(i).is_none_or(|g| *g == eo_render::scene::NO_GROUP)) {
            continue;
        }
        let Some(p) = eo_kit::pipeline::plan_building(rec, &kc.world, i as u32, 0.0) else { continue };
        let cp = eo_kit::clutter::plan(&p, 1);
        if !good(&p, &cp.census) {
            continue;
        }
        let Some(&(_, ei, _, b)) = cp.boxes.iter().find(|b| kinds.contains(&b.0)) else { continue };
        let e = &p.frame.edges[ei];
        let (n, u) = (Vec3::new(e.n.x as f32, 0.0, e.n.z as f32), Vec3::new(e.u.x as f32, 0.0, e.u.z as f32));
        let s = ((b.s0 + b.s1) / 2.0) as f32;
        let m = Vec3::new(e.start.x as f32, 0.0, e.start.z as f32) + u * s;
        let at_xz = m;
        // across the street, a little to one side; far off (`across` past 40 m) along the street instead.
        let eye_xz = if across > 40.0 { m + n * 9.0 + u * across } else { m + n * across + u * (across * 0.3) };
        let ground = |v: Vec3| -> f32 {
            match world.street.as_ref() {
                Some(s) => s.world.surface_at(v.x as f64, v.z as f64, None) as f32,
                None => world.elevation.ground(v.x as f64, v.z as f64) as f32,
            }
        };
        let eye = Vec3::new(eye_xz.x, ground(eye_xz) + EYE_HEIGHT_M, eye_xz.z);
        let at = Vec3::new(at_xz.x, ground(at_xz) + look_up, at_xz.z);
        let what = format!(
            "#{i} {} ({} storeys, built {}) in {district}: {:?}; from {across} m across its front",
            p.classified.archetype,
            p.plan.storeys.len(),
            eo_kit::appearance::building_appearance(&p.classified.record_id, &p.classified, 1900.0).year_built,
            cp.census
        );
        return Some((look_at(eye, at, aspect, far), what));
    }
    None
}

/// A dressed roof from above and to the side.
fn roof_view(world: &World, kc: &eo_kit::city::KitCity, district: &str, aspect: f32, far: f32) -> Option<(Camera, String)> {
    for (i, rec) in kc.records.iter().enumerate() {
        if kc.excluded.contains(&i) || district_of(rec).as_deref() != Some(district) {
            continue;
        }
        let Some(p) = eo_kit::pipeline::plan_building(rec, &kc.world, i as u32, 0.0) else { continue };
        let cp = eo_kit::clutter::plan(&p, 1);
        let c = &cp.census;
        if !(c.chimneys >= 2 && c.dishes >= 1 && c.vents >= 3 && (3..=6).contains(&p.plan.storeys.len())) {
            continue;
        }
        let e = &p.frame.edges[p.front.index];
        let (m, n) = (Vec3::new(e.midpoint.x as f32, 0.0, e.midpoint.z as f32), Vec3::new(e.n.x as f32, 0.0, e.n.z as f32));
        let g = world.elevation.ground(m.x as f64, m.z as f64) as f32;
        let top = g + p.plan.envelope.roof_top as f32;
        let eye = m + n * 9.0 + Vec3::new(0.0, top + 13.0, 0.0);
        let at = m - n * (p.frame.depth as f32 * 0.45) + Vec3::new(0.0, top, 0.0);
        return Some((look_at(eye, at, aspect, far), format!("#{i} {} roof in {district}: {:?}", p.classified.archetype, c)));
    }
    None
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
    let aspect = color.width() as f32 / color.height() as f32;
    let far = args.far;
    let only = std::env::var("EO_CLUTTER_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    let hour = std::env::var("EO_CLUTTER_SHOTS_HOUR").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(15.0);
    let mut list: Vec<(String, Camera, String)> = vec![];
    for (path, frames) in [("kit_street", [60usize, 260]), ("authored_street", [90, 240])] {
        if let Some(s) = segs.iter().find(|s| s.name == path) {
            for f in frames {
                list.push((format!("{path}-f{f}"), (s.camera)(f, args.warmup + s.frames), format!("bench path {path}, frame {f}")));
            }
        }
    }
    let tenancy = eo_kit::tenancy::Tenancy::load(&world.paths.repo_root).map_err(anyhow::Error::msg)?;
    let kc = eo_kit::city::KitCity::build(&world.city, &tenancy, &eo_kit::city::load_exclusions());
    let escapes = |p: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.fire_escapes >= 1 && p.plan.storeys.len() >= 4;
    let shops = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.awnings + c.blade_signs + c.sign_faces + c.shutters >= 4 && c.awnings + c.blade_signs >= 1;
    let esc: &[&str] = &["fire-escape"];
    let shop: &[&str] = &["awning", "blade-sign", "sign-face", "shutter"];
    let blade = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.blade_signs >= 1;
    let awning = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.awnings >= 1;
    let services = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.meter_sets >= 1 && c.downspouts >= 1;
    // (stage S1) a lettered fascia close up, and a neon sign in a shop window.
    let signed = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.sign_faces >= 1 && c.lettered >= 1;
    let windowed = |_: &eo_kit::pipeline::Planned, c: &eo_kit::clutter::Census| c.window_signs >= 1;
    for (name, district, good, kinds, across, up) in [
        ("escape-tenderloin", "Tenderloin", &escapes as &dyn Fn(&eo_kit::pipeline::Planned, &eo_kit::clutter::Census) -> bool, esc, 16.0f32, 8.0f32),
        ("escape-nob-hill", "Nob Hill", &escapes, esc, 16.0, 8.0),
        ("escape-chinatown", "Chinatown", &escapes, esc, 13.0, 7.0),
        ("shops-mission", "Mission", &shops, shop, 11.0, 3.4),
        ("shops-castro", "Castro/Upper Market", &shops, shop, 11.0, 3.4),
        ("shops-chinatown", "Chinatown", &shops, shop, 10.0, 3.6),
        ("blade-mission", "Mission", &blade, &["blade-sign"][..], 8.0, 4.5),
        ("awning-castro", "Castro/Upper Market", &awning, &["awning"][..], 6.0, 2.8),
        ("services-mission", "Mission", &services, &["meters"][..], 4.5, 1.3),
        ("escape-far-tenderloin", "Tenderloin", &escapes, esc, 110.0, 9.0),
        ("sign-close-castro", "Castro/Upper Market", &signed, &["sign-face"][..], 6.0, 3.8),
        ("sign-close-chinatown", "Chinatown", &signed, &["sign-face"][..], 8.0, 3.8),
        ("window-sign-mission", "Mission", &windowed, &["window-sign"][..], 4.5, 2.2),
        ("window-sign-castro", "Castro/Upper Market", &windowed, &["window-sign"][..], 5.0, 2.2),
    ] {
        if !want(name) {
            continue;
        }
        match front_view(world, &kc, district, good, kinds, across, up, aspect, far) {
            Some((cam, what)) => list.push((name.to_string(), cam, what)),
            None => println!("clutter-shots: no {name} card ({district}: nothing dressed found)"),
        }
    }
    if want("roof-mission") {
        if let Some((cam, what)) = roof_view(world, &kc, "Mission", aspect, far) {
            list.push(("roof-mission".into(), cam, what));
        }
    }
    for (name, cam, what) in list {
        if !want(&name) {
            continue;
        }
        renderer.look.set_time_of_day(hour);
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        let p = dir.join(format!("capture-clutter-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        println!("wrote {} (eye {:.1} {:.2} {:.1}, {hour} h: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
    }
    renderer.look.set_time_of_day(args.time_of_day);
    println!("clutter-shots: {}", crate::quality::facade_clutter_json(args));
    Ok(())
}
