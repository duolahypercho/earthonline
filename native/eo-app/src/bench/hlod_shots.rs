//! the far layer's matched captures (`--bench --hlod-shots --label NAME`,
//! with `--no-hlod` for the before side): the skyline from the Bay Bridge's
//! deck, downtown across the bay from 1 km out (the water slice's
//! `skyline-reflection` card), the elevated sweep over the city (two frames of
//! the `elevated_sweep` path), a hill view toward downtown from the highest
//! ground in the city, and the city from above over the western hills (the
//! sky slice's `above` card), each at noon, golden hour and night. Every shot
//! is settled (cells around the camera and every district resident, with the
//! far layer on) before the frames are drawn; the camera, hour and settings
//! of every shot are written to `capture-hlod-<label>-manifest.json`.
//! `EO_HLOD_SHOTS=a,b` keeps only the shots whose name contains a or b.

use super::{Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{Renderer, Targets, wgpu};
use glam::Vec3;

pub const HOURS: [(&str, f64); 3] = [("noon", 12.0), ("golden", 18.5), ("night", 21.0)];

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--hlod-shots")
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

/// Downtown: the mean footprint centre of the buildings over 100 m tall.
pub fn downtown(world: &World) -> Vec3 { // pub
    let (mut s, mut n) = (Vec3::ZERO, 0.0f32);
    for b in &world.city.buildings {
        if b.height > 100.0 && !b.ring.is_empty() {
            let c = b.ring.iter().fold(Vec3::ZERO, |a, p| a + Vec3::new(p[0], 0.0, p[1])) / b.ring.len() as f32;
            s += c;
            n += 1.0;
        }
    }
    let c = if n > 0.0 { s / n } else { super::on_plan(Vec3::new(2000.0, 0.0, -1500.0)) };
    Vec3::new(c.x, 60.0, c.z)
}

/// The highest ground on a 50 m grid over the buildings' extent within 4 km of
/// `near` (Twin Peaks for downtown: Mount Davidson, higher, sees it only past them).
fn hill(world: &World, near: Vec3) -> Vec3 {
    hill_within(world, near, 4000.0)
}

/// the highest ground on a 50 m grid within `reach` metres of `near`.
pub fn hill_within(world: &World, near: Vec3, reach: f64) -> Vec3 {
    let g = &world.elevation;
    let mut best = (f64::MIN, 0.0f64, 0.0f64);
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for b in &world.city.buildings {
        for p in &b.ring {
            x0 = x0.min(p[0]);
            z0 = z0.min(p[1]);
            x1 = x1.max(p[0]);
            z1 = z1.max(p[1]);
        }
    }
    let mut z = z0 as f64;
    while z < z1 as f64 {
        let mut x = x0 as f64;
        while x < x1 as f64 {
            let near_enough = (x - near.x as f64).hypot(z - near.z as f64) <= reach;
            if let Some(h) = g.sample(x, z).filter(|_| near_enough) {
                if h > best.0 {
                    best = (h, x, z);
                }
            }
            x += 50.0;
        }
        z += 50.0;
    }
    Vec3::new(best.1 as f32, best.0 as f32, best.2 as f32)
}

/// The shots: (name, camera, what).
pub fn cameras(world: &World, segs: &[Segment], args: &Args, aspect: f32) -> Vec<(String, Camera, String)> {
    let far = args.far;
    let mut out = vec![];
    let dt = downtown(world);
    // The skyline from the Bay Bridge: beside the west span 1.5 km out from the
    // San Francisco landing, 40 m off its axis on the bay side (north), 75 m up.
    let b = &eo_landmarks::LandmarksJs::get().bay_bridge_west;
    let sf = &b.deck_ends["sf"];
    let landing = Vec3::new(sf.x as f32, 0.0, sf.z as f32);
    let w3 = Vec3::new(b.towers[1].centre[0] as f32, 0.0, b.towers[1].centre[1] as f32);
    let along = (w3 - landing).normalize_or(Vec3::X);
    let side = Vec3::new(-along.z, 0.0, along.x);
    let north = if side.z < 0.0 { side } else { -side };
    let eye = landing + along * 1500.0 + north * 40.0 + Vec3::new(0.0, 75.0, 0.0);
    out.push(("bay-bridge-skyline".to_string(), look_at(eye, dt + Vec3::new(0.0, 40.0, 0.0), aspect, far), format!("beside the Bay Bridge west span 1.5 km out from the San Francisco landing, 40 m north of its axis, 75 m up, looking at downtown ({:.0}, {:.0})", dt.x, dt.z)));
    // Downtown across the bay (the water slice's card).
    for (n, cam, what) in super::water_bench::capture_cameras(world, aspect, far) {
        if n == "skyline-reflection" {
            out.push(("across-the-bay".to_string(), cam, what));
        }
    }
    // The elevated sweep: two frames of the path.
    if let Some(s) = segs.iter().find(|s| s.name == "elevated_sweep") {
        for f in [0usize, 341] {
            out.push((format!("elevated-sweep-f{f}"), (s.camera)(f, args.warmup + s.frames), format!("bench path elevated_sweep, frame {f}")));
        }
    }
    // A hill view toward downtown.
    let h = hill(world, dt);
    let eye = h + Vec3::new(0.0, 25.0, 0.0);
    out.push(("hill-view".to_string(), look_at(eye, dt, aspect, far), format!("25 m over the highest ground within 4 km of downtown ({:.0}, {:.0}, ground {:.0} m), looking at downtown", h.x, h.z, h.y)));
    // The city from above over the western hills (the sky slice's card).
    let from = super::on_plan(Vec3::new(-3600.0, 650.0, 3300.0));
    out.push(("city-above".to_string(), look_at(from, super::on_plan(Vec3::new(1800.0, 0.0, -1200.0)), aspect, far.max(9000.0) * super::gsf()), "the sky slice's `above` card: 650 m over the western hills toward downtown, far plane 9 km".to_string()));
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
    let aspect = color.width() as f32 / color.height() as f32;
    let only = std::env::var("EO_HLOD_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    // Pinned water and cloud clocks so on/off pairs match (see far_shots / so_shots).
    if let Some(w) = renderer.water.get() {
        w.set_time(20.0);
        w.set_always(true);
    }
    let base_clock = args.look.sky.world_time_s;
    let mut manifest = vec![];
    for (hname, hour) in HOURS {
        for (name, cam, what) in cameras(world, segs, args, aspect) {
            let n = format!("{name}-{hname}");
            if !want(&n) {
                continue;
            }
            let t = std::time::Instant::now();
            renderer.look.set_time_of_day(hour);
            renderer.look.sky.set_clock(base_clock, 0.0);
            let settle_ms = settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 900.0)?;
            crate::props::sync_parked(ctx, renderer, props, agents, &cam);
            let p = dir.join(format!("capture-hlod-{label}-{n}.png"));
            capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
            let hl = renderer.hlod.get().map(|h| h.readback(&ctx.queue));
            println!("wrote {} (eye {:.1} {:.1} {:.1}, {hour} h; settle {settle_ms:.0} ms, {:.1} s; far entries drawn/triangles {:?})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, t.elapsed().as_secs_f64(), hl.map(|c| (c[0], c[1])));
            manifest.push(serde_json::json!({
                "file": p.file_name().map(|f| f.to_string_lossy().to_string()),
                "name": name, "hour": hour, "what": what,
                "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "fov_y_deg": cam.fov_y_deg, "near": cam.near, "far": cam.far, "aspect": cam.aspect,
                "settle_ms": settle_ms,
                "far_layer_entries_drawn_triangles_in_frustum_occluded": hl,
            }));
        }
    }
    renderer.look.sky.set_clock(renderer.look.sky.world_time(), 1.0 / 60.0);
    let settings = serde_json::json!({
        "label": label,
        "args": std::env::args().collect::<Vec<_>>(),
        "width": color.width(), "height": color.height(), "msaa": args.msaa,
        "far_layer_on": renderer.hlod.get().is_some(),
        "far_layer": models.hlod.as_ref().map(|h| h.stats_json()),
        "look": format!("{:?}", args.look),
        "shots": manifest,
    });
    let p = dir.join(format!("capture-hlod-{label}-manifest.json"));
    std::fs::write(&p, serde_json::to_string_pretty(&settings)?)?;
    println!("wrote {}", p.display());
    Ok(())
}
