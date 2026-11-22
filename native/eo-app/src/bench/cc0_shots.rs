//! the CC0 slice's matched capture set (`--bench --cc0-shots --out DIR
//! --label after`, and the same with `--no-cc0 --label before`: one binary,
//! a fresh boot each, the same cameras, hours and settings): kit_street frame
//! 260 at noon and golden hour, sidewalk_walk frame 195, one close wall per
//! finish class (fix3's walls), the asphalt, kerb and sidewalk up close, a
//! plaza, a park, a beach, the seawalls, a pier deck and the city from above.
//! `EO_CC0_SHOTS=a,b` limits the set; `EO_CC0_SHOTS_HOUR=H` takes every card
//! at that hour. Files: `capture-cc0-<label>-<shot>.png`.

use super::{EYE_HEIGHT_M, Segment, capture, kit_street, on_plan, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{Renderer, Targets, wgpu};
use glam::Vec3;

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--cc0-shots")
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

/// Standing on the footway of the run nearest `near`, at the middle of the
/// walk, looking down across the kerb into the carriageway.
fn kerb_closeup(world: &World, near: Vec3, aspect: f32, far: f32) -> Option<(Camera, String)> {
    let s = world.street.as_ref()?;
    let w = &s.world;
    let mut best: Option<(f32, usize, usize)> = None;
    for (ei, g) in w.geoms.iter().enumerate() {
        let e = &w.plan.entries[ei];
        if !(e.walks[0] >= w.o.min_sidewalk_width) {
            continue;
        }
        for (si, f) in g.stations.iter().enumerate() {
            let d = ((f.x as f32 - near.x).powi(2) + (f.z as f32 - near.z).powi(2)).sqrt();
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, ei, si));
            }
        }
    }
    let (_, ei, si) = best?;
    let e = &w.plan.entries[ei];
    let f = &w.geoms[ei].stations[si];
    let u = e.half + e.walks[0] * 0.55;
    let p = f.at(u);
    let y = w.surface_at(p[0], p[1], None) as f32;
    let eye = Vec3::new(p[0] as f32, y + EYE_HEIGHT_M, p[1] as f32);
    // Toward the carriageway, 3.5 m ahead along the walk, the kerb about 2 m out.
    let to_road = Vec3::new(-f.nx as f32, 0.0, -f.nz as f32);
    let along = Vec3::new(f.tx as f32, 0.0, f.tz as f32);
    let at = eye + to_road * 2.2 + along * 2.4 - Vec3::new(0.0, EYE_HEIGHT_M, 0.0);
    Some((look_at(eye, at, aspect, far), format!("{} (run {}): on the walk {:.1} m from the centreline, looking down across the kerb", e.highway, e.id, u)))
}

/// cc0 v2: on the walk of the run nearest `near`, looking down along the walk
/// at the scored squares 1.5-6 m ahead (the slabs' own variation, the joints).
fn sidewalk_closeup(world: &World, near: Vec3, aspect: f32, far: f32) -> Option<(Camera, String)> {
    let s = world.street.as_ref()?;
    let w = &s.world;
    let mut best: Option<(f32, usize, usize)> = None;
    for (ei, g) in w.geoms.iter().enumerate() {
        let e = &w.plan.entries[ei];
        if !(e.walks[0] >= w.o.min_sidewalk_width + 1.0) {
            continue;
        }
        for (si, f) in g.stations.iter().enumerate() {
            let d = ((f.x as f32 - near.x).powi(2) + (f.z as f32 - near.z).powi(2)).sqrt();
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, ei, si));
            }
        }
    }
    let (_, ei, si) = best?;
    let e = &w.plan.entries[ei];
    let f = &w.geoms[ei].stations[si];
    let u = e.half + e.walks[0] * 0.5;
    let p = f.at(u);
    let y = w.surface_at(p[0], p[1], None) as f32;
    let eye = Vec3::new(p[0] as f32, y + EYE_HEIGHT_M, p[1] as f32);
    let along = Vec3::new(f.tx as f32, 0.0, f.tz as f32);
    let at = eye + along * 3.2 - Vec3::new(0.0, EYE_HEIGHT_M, 0.0);
    Some((look_at(eye, at, aspect, far), format!("{} (run {}): standing mid-walk {:.1} m from the centreline, looking down along the walk", e.highway, e.id, u)))
}

/// cc0 v2: over a travel lane's outer wheel path 20 m short of an
/// intersection approach (a run that has one and parking lanes, nearest
/// `near`), at eye height, looking along the lane toward the approach: the
/// wheel paths, tyre tracks, drips, covers and the gutter.
fn wheel_path(world: &World, near: Vec3, aspect: f32, far: f32) -> Option<(Camera, String)> {
    let s = world.street.as_ref()?;
    let w = &s.world;
    let mut best: Option<(f32, usize)> = None;
    for (ei, g) in w.geoms.iter().enumerate() {
        let e = &w.plan.entries[ei];
        let (Some(last), true) = (g.stations.last(), e.approach_end.is_some() && e.walks[1] >= w.o.min_sidewalk_width) else { continue };
        if last.s - g.stations[0].s < 45.0 || e.oneway || !w.lines[ei].iter().any(|l| l.role == "parking") {
            continue;
        }
        let d = ((last.x as f32 - near.x).powi(2) + (last.z as f32 - near.z).powi(2)).sqrt();
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, ei));
        }
    }
    let (_, ei) = best?;
    let e = &w.plan.entries[ei];
    let g = &w.geoms[ei];
    let end = g.stations.last()?.s;
    let si = g.stations.partition_point(|f| f.s < end - 20.0).min(g.stations.len() - 1);
    let f = &g.stations[si];
    // The -n side travels toward the run's end (right-hand traffic): its outer lane's kerbside wheel path.
    let parking = w.lines[ei].iter().filter(|l| l.role == "parking").map(|l| l.u.abs()).fold(f64::INFINITY, f64::min);
    let u = -(parking - 1.6 - eo_render::cc0_decals::HALF_TRACK_M);
    let p = f.at(u);
    let y = w.surface_at(p[0], p[1], None) as f32;
    let eye = Vec3::new(p[0] as f32, y + EYE_HEIGHT_M, p[1] as f32);
    let along = Vec3::new(f.tx as f32, 0.0, f.tz as f32);
    let at = eye + along * 9.0 - Vec3::new(0.0, EYE_HEIGHT_M, 0.0);
    Some((look_at(eye, at, aspect, far), format!("{} (run {}): over the kerbside wheel path {:.1} m from the centreline, 20 m short of the approach, looking along the lane", e.highway, e.id, u)))
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
    let only = std::env::var("EO_CC0_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    let mut list: Vec<(String, Camera, f64, String)> = vec![];
    let seg = |n: &str| segs.iter().find(|s| s.name == n);
    if let Some(s) = seg("kit_street") {
        let cam = (s.camera)(260, args.warmup + s.frames);
        list.push(("kit_street-f260-noon".into(), cam, 12.0, "bench path kit_street, frame 260".into()));
        list.push(("kit_street-f260-golden".into(), cam, 18.5, "bench path kit_street, frame 260, golden hour".into()));
        list.push(("kit_street-f260-night".into(), cam, 21.0, "bench path kit_street, frame 260, night (the sky slice night hour)".into()));
    }
    if let Some(s) = seg("sidewalk_walk") {
        list.push(("sidewalk_walk-f195-noon".into(), (s.camera)(195, args.warmup + s.frames), 12.0, "bench path sidewalk_walk, frame 195".into()));
    }
    for (name, cam, what) in super::fix3_shots::cameras(world, aspect, far) {
        list.push((name, cam, 12.0, what));
    }
    let ks = kit_street();
    let mid = Vec3::new((ks[0][0] + ks[1][0]) / 2.0, 0.0, (ks[0][1] + ks[1][1]) / 2.0);
    if let Some((cam, what)) = kerb_closeup(world, mid, aspect, far) {
        list.push(("kerb-closeup".into(), cam, 12.0, what));
    }
    if let Some((cam, what)) = sidewalk_closeup(world, mid, aspect, far) {
        list.push(("sidewalk-closeup".into(), cam, 12.0, what));
    }
    if let Some((cam, what)) = wheel_path(world, mid, aspect, far) {
        list.push(("wheel-path".into(), cam, 12.0, what.clone()));
        list.push(("wheel-path-golden".into(), cam, 18.5, what));
    }
    for (name, cam, what) in super::landmarks_bench::capture_cameras(world, aspect, far) {
        if matches!(name, "plaza" | "park" | "ocean-beach-seawall" | "embarcadero-seawall" | "aquatic-park" | "pier-deck") {
            list.push((name.to_string(), cam, 12.0, what));
        }
    }
    // Ocean Beach: on the sand 55 m seaward of the Esplanade, looking south along the beach.
    let ob = &eo_landmarks::LandmarksJs::get().promenades["oceanBeach"]["ESPLANADE"];
    if let (Some(ax), Some(az), Some(bx), Some(bz)) = (ob[0]["x"].as_f64(), ob[0]["z"].as_f64(), ob[1]["x"].as_f64(), ob[1]["z"].as_f64()) {
        let (a, b) = (Vec3::new(ax as f32, 0.0, az as f32), Vec3::new(bx as f32, 0.0, bz as f32));
        let d = (b - a).normalize();
        let sea_n = Vec3::new(-d.z, 0.0, d.x);
        let on = a + d * ((b - a).length() * 0.4) + sea_n * 55.0;
        let g = world.elevation.ground(on.x as f64, on.z as f64) as f32;
        let eye = Vec3::new(on.x, g.max(eo_water::SEA_LEVEL_Y as f32) + EYE_HEIGHT_M, on.z);
        let at = eye + d * 30.0 + sea_n * 6.0 - Vec3::new(0.0, 4.5, 0.0);
        list.push(("beach".into(), look_at(eye, at, aspect, far), 12.0, "Ocean Beach from the sand 55 m seaward of the Esplanade, looking along the beach".into()));
    }
    // The city from above (the sky slice's camera), at noon.
    let from = on_plan(Vec3::new(-3600.0, 650.0, 3300.0));
    let to = on_plan(Vec3::new(1800.0, 0.0, -1200.0)) - from;
    let above = Camera { far: far.max(9000.0), ..Camera::new(from, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length()).atan(), aspect) };
    list.push(("above-noon".into(), above, 12.0, "the city from over the western hills toward downtown".into()));
    // `EO_CC0_SHOTS_HOUR=H`: every card at that hour (unset: each card's own).
    let hour_over = std::env::var("EO_CC0_SHOTS_HOUR").ok().and_then(|v| v.parse::<f64>().ok());
    for (name, cam, hour, what) in list {
        if !want(&name) {
            continue;
        }
        let hour = hour_over.unwrap_or(hour);
        renderer.look.set_time_of_day(hour);
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        let p = dir.join(format!("capture-cc0-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        println!("wrote {} (eye {:.1} {:.2} {:.1}, {hour} h: {what})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z);
    }
    renderer.look.set_time_of_day(args.time_of_day);
    println!("cc0: {}", models.cc0.as_ref().map(|c| c.report().to_string()).unwrap_or_else(|| "off".into()));
    Ok(())
}
