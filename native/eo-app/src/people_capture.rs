//! matched captures of the street people and the people pass's cost
//! at four poses (offscreen):
//!
//!   eo-app --people-capture [--no-rocketbox] [--no-rocketbox-motions] [--label NAME] [--out DIR] [--people-frames 240] [--people-lineup] [--people-strip N]
//!
//! The simulation is rebuilt and stepped a fixed 600 frames of 1/60 s from
//! its warm-up (nothing posted while loading), then held still (dt 0), so
//! every run with the same build and flags puts the same walkers in the same
//! places; the snapshot's digest is printed to check it. The poses (a busy
//! sidewalk at 3-8 m, a sidewalk at 20-40 m, a crossing, and the first pose
//! at 21 h) are chosen from that snapshot and written to
//! `people-poses-<label>.json`; `EO_PEOPLE_POSES=<file>` renders a pose file
//! instead, so the other side of an A/B reuses the same cameras. Per pose:
//! `capture-people-<label>-<pose>.png`, and the frame timed (CPU + GPU, the
//! bench's definition) with every agent, with the simulated people left out,
//! and with no agents, two rounds (`EO_PEOPLE_ROUNDS`) in alternating order; the people pass is
//! the first minus the second. `--people-lineup` adds review shots of eight
//! pool people side by side (walking and standing) from the front and side
//! (and idle and phone close-ups, and eight walkers in six
//! frames 0.1 s apart); `--people-strip N` the first pose N times, the
//! simulation stepped 0.1 s between them.

use crate::agents::AgentRuntime;
use crate::agents_bench::{PeopleGpu, Run, clear_view, heading_of, look_at, measure_dt, percentile, surface};
use crate::models::ModelRuntime;
use crate::{Args, World};
use anyhow::{Result, anyhow};
use eo_agents::{CLASS_PERSON, CLIP_IDLE, GpuAgent};
use eo_render::camera::Camera;
use eo_render::{read_mapped, wgpu};
use glam::{Quat, Vec3};
use serde_json::json;
use std::path::Path;

const FRAME_S: f64 = 1.0 / 60.0;
const SIM_FRAMES: usize = 600;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn frame(g: &PeopleGpu, cam: &Camera) {
    g.renderer.update(&g.ctx.queue, cam);
    let mut enc = g.ctx.device.create_command_encoder(&Default::default());
    let (attach, resolve) = g.attach();
    g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
    g.ctx.queue.submit([enc.finish()]);
    let _ = g.ctx.device.poll(wgpu::PollType::wait_indefinitely());
}

/// Pump with dt 0 until the instance list stops changing (the feed caught up).
fn hold(g: &PeopleGpu, agents: &mut AgentRuntime, cam: &Camera) {
    let mut last: Vec<GpuAgent> = vec![];
    let mut same = 0;
    for _ in 0..400 {
        agents.pump(g.ctx, g.renderer, AgentRuntime::focus_of(cam.eye, cam.forward()), 0.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let now = agents.snapshot();
        if !now.is_empty() && now == last {
            same += 1;
            if same >= 4 {
                return;
            }
        } else {
            same = 0;
        }
        last = now;
    }
}

/// Stream the models, kit and props for this camera (agents_bench's settle).
fn settle(g: &PeopleGpu, world: &World, models: &mut ModelRuntime, cam: &Camera) -> Result<()> {
    let t = std::time::Instant::now();
    models.set_camera(cam.eye);
    loop {
        models.pump(g.ctx, g.renderer, world, cam.eye)?;
        crate::props::global_pump(g.ctx, g.renderer, &mut models.uploads, cam.eye);
        g.renderer.update(&g.ctx.queue, cam);
        g.ctx.queue.submit([]);
        let _ = g.ctx.device.poll(wgpu::PollType::Poll);
        if (models.settled() && crate::props::global_settled()) || t.elapsed().as_secs_f64() > 600.0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn capture(g: &PeopleGpu, cam: &Camera, path: &Path) -> Result<()> {
    let ctx = g.ctx;
    let (w, h) = (g.color.width(), g.color.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("people-capture"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    // 20 frames: every cascade and the probe refreshed for this camera, the temporal resolve settled.
    for _ in 0..20 {
        frame(g, cam);
    }
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        g.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let bytes = read_mapped(ctx, &buf)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + w as usize * 4]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(file, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn pose_json(name: &str, c: &Camera, hour: f64, what: &str) -> serde_json::Value {
    json!({ "name": name, "eye": [c.eye.x, c.eye.y, c.eye.z], "yaw": c.yaw, "pitch": c.pitch, "fov_y_deg": c.fov_y_deg, "near": c.near, "far": c.far, "aspect": c.aspect, "hour": hour, "what": what })
}

fn read_poses(p: &Path) -> Result<Vec<(String, Camera, f64, String)>> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p)?)?;
    let f = |x: &serde_json::Value| x.as_f64().unwrap_or(0.0) as f32;
    Ok(v.as_array()
        .ok_or_else(|| anyhow!("pose file is not an array"))?
        .iter()
        .map(|p| {
            let e = &p["eye"];
            (
                p["name"].as_str().unwrap_or("pose").to_string(),
                Camera { eye: Vec3::new(f(&e[0]), f(&e[1]), f(&e[2])), yaw: f(&p["yaw"]), pitch: f(&p["pitch"]), fov_y_deg: f(&p["fov_y_deg"]), near: f(&p["near"]), far: f(&p["far"]), aspect: f(&p["aspect"]), jitter: [0.0; 2], roll: 0.0 },
                p["hour"].as_f64().unwrap_or(12.0),
                p["what"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect())
}

fn digest(list: &[GpuAgent]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for a in list {
        let g = GpuAgent { model: 0, color: 0, ..*a };
        for b in bytemuck::bytes_of(&g) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

pub fn run(g: &PeopleGpu, world: &mut World, args: &Args, models: &mut ModelRuntime, agents: &mut AgentRuntime, path: &[Camera], dir: &Path) -> Result<()> {
    let label = args.label.clone().unwrap_or_else(|| if args.rocketbox { "rocketbox".into() } else { "no-people".into() });
    let lib = agents.lib.clone().ok_or_else(|| anyhow!("no agent library"))?;
    let is_person = |a: &GpuAgent| lib.models.get(a.model as usize).is_some_and(|m| m.class == CLASS_PERSON);
    let cam0 = path[0];
    let focus = AgentRuntime::focus_of(cam0.eye, cam0.forward());
    let aspect = g.color.width() as f32 / g.color.height() as f32;
    // A fresh simulation from its warm-up, nothing posted while it loads.
    agents.restart_sim(world, focus, agents.opts.density);
    let t = std::time::Instant::now();
    while !agents.ready() && t.elapsed().as_secs_f64() < 300.0 {
        agents.pump(g.ctx, g.renderer, focus, 0.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if !agents.ready() {
        return Err(anyhow!("agents not ready: {:?}", agents.errors));
    }
    for _ in 0..SIM_FRAMES {
        agents.pump(g.ctx, g.renderer, focus, FRAME_S);
        frame(g, &cam0);
    }
    hold(g, agents, &cam0);
    let snap = agents.snapshot();
    let people: Vec<GpuAgent> = snap.iter().copied().filter(|a| is_person(a)).collect();
    println!(
        "people-capture {label}: library {} ({}{}), {} agents ({} people) after {} frames; snapshot digest {} (people positions {})",
        lib.meta.people_source,
        if lib.from_cache { "from the cache" } else { "cooked" },
        lib.meta.people_fallback.as_deref().map(|f| format!(", fallback: {f}")).unwrap_or_default(),
        snap.len(),
        people.len(),
        SIM_FRAMES,
        digest(&snap),
        // where they are and what they do, not how the motion is phased.
        digest(&people.iter().map(|a| GpuAgent { model: 0, anim: 0.0, clip: 0, clip_phase: 0.0, ..*a }).collect::<Vec<_>>())
    );
    let p = |a: &GpuAgent| Vec3::from(a.pos);
    let mut poses: Vec<(String, Camera, f64, String)> = match std::env::var("EO_PEOPLE_POSES").ok() {
        Some(f) => read_poses(Path::new(&f))?,
        None => {
            let mut out = vec![];
            let fov = eo_render::camera::GAME_FOV_Y_DEG;
            use eo_sim::output::{FLAG_CROSSING, FLAG_KERB_WAIT};
            let walkers: Vec<&GpuAgent> = people.iter().filter(|a| a.flags & FLAG_CROSSING as u32 == 0 && p(a).distance(cam0.eye) < 150.0).collect();
            // The busiest spot: the walker with the most others within 4 m.
            let busiest = walkers.iter().max_by_key(|a| walkers.iter().filter(|b| p(b).distance(p(a)) < 4.0).count());
            if let Some(a) = busiest {
                let group: Vec<Vec3> = walkers.iter().filter(|b| p(b).distance(p(a)) < 4.0).map(|b| p(b)).collect();
                let c = group.iter().sum::<Vec3>() / group.len() as f32;
                let n = group.len();
                out.push(("sidewalk-near".to_string(), clear_view(world, c, heading_of(a) + 0.5, 5.5, 1.65, 1.1, args.far, aspect, fov), args.time_of_day, format!("{n} people within 4 m of the aim; camera 5.5 m away")));
                out.push(("sidewalk-far".to_string(), clear_view(world, c, heading_of(a) + 0.9, 30.0, 1.7, 1.0, args.far, aspect, fov), args.time_of_day, format!("the same group from 30 m")));
            }
            let crossing: Vec<&GpuAgent> = people.iter().filter(|a| a.flags & (FLAG_CROSSING | FLAG_KERB_WAIT) as u32 != 0).collect();
            if let Some(a) = crossing.iter().min_by(|a, b| p(a).distance(cam0.eye).total_cmp(&p(b).distance(cam0.eye))) {
                let n = crossing.iter().filter(|b| p(b).distance(p(a)) < 15.0).count();
                out.push(("crossing".to_string(), clear_view(world, p(a), heading_of(a) + 1.6, 14.0, 3.0, 1.0, args.far, aspect, fov), args.time_of_day, format!("{n} people crossing or waiting within 15 m")));
            }
            if let Some(first) = out.first().cloned() {
                out.push(("night".to_string(), first.1, 21.0, format!("{} at 21 h", first.0)));
            }
            out
        }
    };
    let pj: Vec<serde_json::Value> = poses.iter().map(|(n, c, h, w)| pose_json(n, c, *h, w)).collect();
    let first_pose = poses.first().cloned();
    let pf = dir.join(format!("people-poses-{label}.json"));
    std::fs::write(&pf, serde_json::to_string_pretty(&pj)?)?;
    println!("wrote {}", pf.display());
    let frames: usize = arg("--people-frames").and_then(|v| v.parse().ok()).unwrap_or(240);
    let period_ns = g.ctx.queue.get_timestamp_period() as f64;
    let saved_hour = g.renderer.look.time_of_day();
    let mut table = vec![];
    if std::env::var("EO_PEOPLE_LINEUP_ONLY").is_ok() {
        poses.clear();
    }
    for (name, cam, hour, what) in poses.drain(..) {
        g.renderer.look.set_time_of_day(hour);
        settle(g, world, models, &cam)?;
        agents.parked = crate::props::global_parked(agents.lib.as_deref(), cam.eye);
        hold(g, agents, &cam);
        capture(g, &cam, &dir.join(format!("capture-people-{label}-{name}.png")))?;
        let path: Vec<Camera> = vec![cam; 30 + frames];
        let mut runs: Vec<(&str, Run)> = vec![];
        let rounds: usize = std::env::var("EO_PEOPLE_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        for round in 0..rounds {
            let order: [&str; 3] = if round % 2 == 0 { ["all", "no-people", "off"] } else { ["off", "no-people", "all"] };
            for mode in order {
                agents.hide_people = mode == "no-people";
                let r = measure_dt(g, agents, &path, mode != "off", period_ns, 0.0)?;
                runs.push((mode, r));
            }
        }
        agents.hide_people = false;
        agents.opts.on = true;
        if let Some(a) = g.renderer.agents.get() {
            a.set_enabled(true);
        }
        let med = |mode: &str, f: &dyn Fn(&Run) -> &Vec<f64>| -> f64 {
            let v: Vec<f64> = runs.iter().filter(|(m, _)| *m == mode).map(|(_, r)| percentile(f(r), 50.0)).collect();
            percentile(&v, 50.0)
        };
        let row = json!({
            "pose": name, "hour": hour, "what": what, "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
            "frame_all": med("all", &|r| &r.total), "frame_no_people": med("no-people", &|r| &r.total), "frame_off": med("off", &|r| &r.total),
            "frame_all_p95": runs.iter().filter(|(m, _)| *m == "all").map(|(_, r)| percentile(&r.total, 95.0)).fold(0.0, f64::max),
            "gpu_all": med("all", &|r| &r.gpu), "gpu_no_people": med("no-people", &|r| &r.gpu),
            "city_all": med("all", &|r| &r.city), "city_no_people": med("no-people", &|r| &r.city),
            "shadow_all": med("all", &|r| &r.sh_depth), "shadow_no_people": med("no-people", &|r| &r.sh_depth),
            "cull_all": med("all", &|r| &r.ag_cull), "casters_all": med("all", &|r| &r.ag_cast), "casters_no_people": med("no-people", &|r| &r.ag_cast),
            "cpu_all": med("all", &|r| &r.cpu), "cpu_no_people": med("no-people", &|r| &r.cpu),
            "drawn_all": med("all", &|r| &r.drawn), "drawn_no_people": med("no-people", &|r| &r.drawn),
            "tris_all": med("all", &|r| &r.tris), "tris_no_people": med("no-people", &|r| &r.tris),
            "shadow_tris_all": med("all", &|r| &r.shadow_tris), "shadow_tris_no_people": med("no-people", &|r| &r.shadow_tris),
        });
        let f = |k: &str| row[k].as_f64().unwrap_or(f64::NAN);
        println!(
            "  {name} ({hour} h): frame all {:.3} / no people {:.3} / off {:.3} ms (all p95 {:.3}) -> people pass {:.3} ms (GPU {:.3}, city {:.3}, casters {:.3}, shadow pass {:.3}); people drawn {:.0}, people triangles {:.0} k, people shadow triangles {:.0} k; {what}",
            f("frame_all"), f("frame_no_people"), f("frame_off"), f("frame_all_p95"),
            f("frame_all") - f("frame_no_people"), f("gpu_all") - f("gpu_no_people"), f("city_all") - f("city_no_people"), f("casters_all") - f("casters_no_people"), f("shadow_all") - f("shadow_no_people"),
            f("drawn_all") - f("drawn_no_people"), (f("tris_all") - f("tris_no_people")) / 1e3, (f("shadow_tris_all") - f("shadow_tris_no_people")) / 1e3,
        );
        table.push(row);
    }
    g.renderer.look.set_time_of_day(saved_hour);
    if std::env::args().any(|a| a == "--people-lineup") {
        lineup(g, world, args, models, agents, &cam0, dir, &label)?;
    }
    // the crowd walking, 0.1 s a frame, from the first pose.
    if let Some(n) = arg("--people-strip").and_then(|v| v.parse::<usize>().ok()) {
        if let Some((_, cam, hour, _)) = first_pose.as_ref() {
            strip(g, world, models, agents, cam, *hour, n.clamp(1, 12), dir, &label)?;
        }
    }
    let alloc = g.ctx.device.generate_allocator_report();
    let out = json!({
        "label": label, "people_source": lib.meta.people_source, "people_fallback": lib.meta.people_fallback,
        "library": { "from_cache": lib.from_cache, "load_ms": lib.load_ms, "people_cook_ms": lib.meta.people_cook_ms, "models": lib.models.len(), "meshes": lib.meshes.len(), "vertices": lib.vertices.len(), "triangles": lib.indices.len() / 3, "atlases": lib.atlases.len(), "atlas_res": lib.meta.atlas_res, "bones_bytes": lib.bones.len() * 16, "people_bone_bytes": lib.meta.people_bone_bytes, "people_groups": lib.meta.people_groups, "lib_ready_ms": agents.lib_ready_ms },
        "agents_vram_bytes": g.renderer.agents.get().map(|a| a.bytes),
        "renderer_vram_bytes": g.renderer.bytes.total(),
        "wgpu_allocator_total_allocated": alloc.as_ref().map(|a| a.total_allocated_bytes),
        "snapshot_digest": digest(&snap), "people": people.len(), "frames_per_run": frames,
        "table": table,
    });
    let p = dir.join(format!("people-capture-{label}.json"));
    std::fs::write(&p, serde_json::to_string_pretty(&out)?)?;
    println!("wrote {} (agents VRAM {:.1} MB)", p.display(), g.renderer.agents.get().map(|a| a.bytes as f64 / 1e6).unwrap_or(0.0));
    Ok(())
}

/// Eight pool people side by side, walking (left) and standing (right), from
/// the front and the side; close-ups of two people standing
/// idle and two on a phone call (a woman and a man each), and eight walkers
/// side on (each a different walk style when there are styles) in six
/// frames 0.1 s apart (`lineup-walk-0` .. `-5`). The same phases on both
/// sides of an A/B; the instance phases come from the library's layout.
#[allow(clippy::too_many_arguments)]
fn lineup(g: &PeopleGpu, world: &mut World, args: &Args, models: &mut ModelRuntime, agents: &mut AgentRuntime, cam0: &Camera, dir: &Path, label: &str) -> Result<()> {
    let lib = agents.lib.clone().ok_or_else(|| anyhow!("no agent library"))?;
    let pool = &lib.meta.people_models;
    if pool.is_empty() {
        return Ok(());
    }
    let mo = &lib.meta.motion;
    let fwd = Vec3::new(cam0.forward().x, 0.0, cam0.forward().z).normalize_or(Vec3::Z);
    let right = Vec3::new(fwd.z, 0.0, -fwd.x);
    let c0 = cam0.eye + fwd * 6.0;
    let centre = Vec3::new(c0.x, surface(world, c0.x, c0.z), c0.z);
    let eye0 = cam0.eye + fwd * 0.0;
    let eye = Vec3::new(eye0.x, surface(world, eye0.x, eye0.z) + 1.3, eye0.z);
    // a person for slot k of eight (the pool spread evenly).
    let pick = |k: usize| (k * pool.len() / 8).min(pool.len() - 1);
    let female = |p: usize| lib.meta.model_names.get(pool[p] as usize).is_some_and(|n| n.contains("female"));
    let woman = (0..pool.len()).find(|&p| female(p)).unwrap_or(0);
    let man = (0..pool.len()).find(|&p| !female(p)).unwrap_or(0);
    let style_of = |k: usize| if mo.styled() { (k * mo.styles.len() / 8).min(mo.styles.len() - 1) } else { 0 };
    let person = |p: usize, at: Vec3, facing: Vec3, speed: f32, anim: f32, clip: u32, clip_phase: f32| GpuAgent {
        pos: [at.x, surface(world, at.x, at.z), at.z],
        model: pool[p],
        rot: Quat::from_rotation_y(facing.x.atan2(facing.z)).to_array(),
        color: 0xffffffff,
        flags: 0,
        anim,
        steer: 0.0,
        speed,
        scale: 1.0,
        clip_phase,
        clip,
    };
    let make = |facing: Vec3| -> Vec<GpuAgent> {
        (0..8usize)
            .map(|k| {
                let at = centre + right * ((k as f32 - 3.5) * 0.95);
                let walking = k < 4;
                let anim = mo.walk_anim(style_of(k), (k as f64 * 0.25).fract());
                person(pick(k), at, facing, if walking { 1.4 } else { 0.0 }, anim, CLIP_IDLE, mo.clip_phase(CLIP_IDLE as usize, 0, k as f64 * 0.13))
            })
            .collect()
    };
    agents.hide_people = true;
    let wide = look_at(eye, centre + Vec3::Y * 0.9, args.far, cam0.aspect, 40.0);
    let near_at = centre + right * (-1.5 * 0.95);
    let close = look_at(near_at + fwd * -2.4 + Vec3::Y * 1.5 + right * 0.3, near_at + Vec3::Y * 1.45, args.far, cam0.aspect, 30.0);
    let mut shots: Vec<(String, Vec<GpuAgent>, Camera)> = vec![("front".into(), make(-fwd), wide), ("side".into(), make(right), wide), ("close".into(), make(-fwd), close)];
    // two people idle and two on a call, 1.1 m apart, from 2.6 m.
    let pair_cam = look_at(centre - fwd * 2.6 + Vec3::Y * 1.45, centre + Vec3::Y * 1.2, args.far, cam0.aspect, 35.0);
    let call = (1..4).find_map(|s| mo.loops[s].iter().position(|l| l.role == "call-talk").map(|l| (s, l))).unwrap_or((eo_agents::CLIP_PHONE as usize, 0));
    // The pack's phone at its nearest to the ear (8.4 s of its 12 s); the call loops keep the hand there.
    let phone_at = if mo.looped(call.0) { [0.35, 0.55] } else { [0.70, 0.70] };
    for (name, clip, l, at) in [("idle-close", CLIP_IDLE as usize, 0usize, [0.35, 0.55]), ("phone-close", call.0, call.1, phone_at)] {
        let list = [woman, man].iter().enumerate().map(|(i, &p)| person(p, centre + right * ((i as f32 - 0.5) * 1.1), -fwd, 0.0, 0.0, clip as u32, mo.clip_phase(clip, l, at[i]))).collect();
        shots.push((name.into(), list, pair_cam));
    }
    // eight walkers side on, six frames 0.1 s apart (each
    // advanced by distance over its own stride; today's walk by the sim's).
    // Shifted 1.4 m along `right` (towards the kerb at the spawn: the lineup's far end stands in a wall).
    let mid = centre + right * 1.4;
    let strip_cam = look_at(mid - fwd * 6.5 + Vec3::Y * 1.2, mid + Vec3::Y * 0.85, args.far, cam0.aspect, 40.0);
    for f in 0..6 {
        let list = (0..8usize)
            .map(|k| {
                let p = pick(k);
                let v = 1.3f64;
                let stride = if mo.styled() { mo.walk_of(p, style_of(k))[0] as f64 } else { eo_agents::gait::stride(v, eo_agents::gait::REFERENCE_HEIGHT_M) };
                let phase = 0.1 * k as f64 + f as f64 * 0.1 * v / stride.max(0.2);
                person(p, mid + right * ((k as f32 - 3.5) * 0.8), right, v as f32, mo.walk_anim(style_of(k), phase), CLIP_IDLE, 0.0)
            })
            .collect();
        shots.push((format!("walk-{f}"), list, strip_cam));
    }
    for (n, list, cam) in shots {
        agents.extras = list;
        settle(g, world, models, &cam)?;
        hold(g, agents, &cam);
        capture(g, &cam, &dir.join(format!("capture-people-{label}-lineup-{n}.png")))?;
    }
    agents.extras.clear();
    agents.hide_people = false;
    Ok(())
}

/// `--people-strip N`: the first pose again, N frames, the
/// simulation stepped 0.1 s (six 1/60 s steps) before each (the crowd walking).
#[allow(clippy::too_many_arguments)]
fn strip(g: &PeopleGpu, world: &mut World, models: &mut ModelRuntime, agents: &mut AgentRuntime, cam: &Camera, hour: f64, frames: usize, dir: &Path, label: &str) -> Result<()> {
    g.renderer.look.set_time_of_day(hour);
    settle(g, world, models, cam)?;
    let focus = AgentRuntime::focus_of(cam.eye, cam.forward());
    for f in 0..frames {
        if f > 0 {
            for _ in 0..6 {
                agents.pump(g.ctx, g.renderer, focus, FRAME_S);
                frame(g, cam);
            }
        }
        hold(g, agents, cam);
        capture(g, cam, &dir.join(format!("capture-people-{label}-strip-{f}.png")))?;
    }
    Ok(())
}
