//! the cost of the moving things and captures of them (offscreen).
//!
//!   eo-app --agents-bench [--ab-rounds 3] [--densities 1,4,16] [--ab-frames 400]
//!       Interleaved A/B on one busy street path, same binary and process:
//!       for each population density (1x = 42 moving cars + People `medium`),
//!       `rounds` pairs of runs with agents OFF (no simulation posted, no agent
//!       pass) and ON, the order alternating each round. Prints and writes
//!       bench-out/agents-ab-<stamp>.json: frame total (CPU + GPU as the main
//!       bench sums it), the GPU passes, the agent passes, and the differences.
//!   eo-app --agents-capture
//!       PNGs bench-out/capture-agents-*.png: the street with traffic, a
//!       crossing with people, a close person, a close car, the player on foot
//!       (third person) and in the player's car.
//!
//! The path: eye 1.7 m over the drawn street surface at the spawn on Folsom
//! Street (SoMa, the simulation's downtown focus), walking 1.4 m/s (per frame
//! at a nominal 60 fps) along the street, the view swinging +-60 degrees.

use crate::agents::{AgentRuntime, PlayerVisual};
use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_agents::{CLASS_PERSON, GpuAgent};
use eo_render::camera::Camera;
use eo_render::{GpuContext, GpuOptions, QUERIES_PER_FRAME, Renderer, Targets, read_mapped, wgpu};
use glam::{DVec3, Vec3};
use serde_json::json;
use std::time::Instant;

const EYE_M: f32 = 1.7;
const WALK_M_PER_FRAME: f32 = 1.4 / 60.0;
const FRAME_S: f64 = 1.0 / 60.0;
const WARMUP: usize = 30;

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--agents-bench" || a == "--agents-capture" || a == "--people-capture") // + --people-capture
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

pub(crate) fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({ "median": percentile(v, 50.0), "p95": percentile(v, 95.0), "max": percentile(v, 100.0), "n": v.len() })
}

fn yaw_of(d: Vec3) -> f32 {
    d.x.atan2(-d.z)
}

/// The Folsom Street direction at the spawn (as bench.rs's `folsom_heading`).
fn folsom_heading(world: &World) -> Vec3 {
    let s = glam::Vec2::from(crate::bench::spawn()); // the spawn on the scaled plan
    let mut best = (f32::MAX, glam::Vec2::new(1.0, -1.0));
    for r in world.city.roads.iter().filter(|r| r.name == "Folsom Street") {
        for w in r.points.windows(2) {
            let (a, b) = (glam::Vec2::from(w[0]), glam::Vec2::from(w[1]));
            let ab = b - a;
            let t = ((s - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (a + ab * t).distance(s);
            if d < best.0 {
                best = (d, ab);
            }
        }
    }
    let mut dir = best.1.normalize_or(glam::Vec2::new(1.0, -1.0).normalize());
    if dir.x < 0.0 {
        dir = -dir;
    }
    Vec3::new(dir.x, 0.0, dir.y)
}

pub(crate) fn surface(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

fn inside_building(world: &World, x: f32, z: f32) -> bool {
    world.street.as_ref().is_some_and(|s| s.world.buildings.contains(x as f64, z as f64))
}

struct Gpu {
    ctx: GpuContext,
    renderer: Renderer,
    color: wgpu::Texture,
    view: wgpu::TextureView,
    targets: Targets,
}

impl Gpu {
    fn attach(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match &self.targets.msaa {
            Some(m) => (m, Some(&self.view)),
            None => (&self.view, None),
        }
    }
}

fn settle(g: &Gpu, world: &World, models: &mut ModelRuntime, cam: &Camera) -> Result<()> {
    let t = Instant::now();
    models.set_camera(cam.eye);
    loop {
        models.pump(&g.ctx, &g.renderer, world, cam.eye)?;
        crate::props::global_pump(&g.ctx, &g.renderer, &mut models.uploads, cam.eye); // props
        g.renderer.update(&g.ctx.queue, cam);
        g.ctx.queue.submit([]);
        let _ = g.ctx.device.poll(wgpu::PollType::Poll);
        if (models.settled() && crate::props::global_settled()) || t.elapsed().as_secs_f64() > 600.0 { // props
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    g.ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

pub fn run(args: &Args, process_start: Instant) -> Result<()> {
    let mut world = load_world(args)?;
    let opts = GpuOptions { indirect_validation: args.indirect_validation, force_no_indirect_count: args.no_indirect_count, force_no_count_emulation: args.no_count_emulation, bounded_draws: args.bounded_draws, ..Default::default() };
    let ctx = GpuContext::new(GpuContext::create_instance(&opts), None, opts)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (renderer, bases) = make_renderer(&ctx, &world, args, format, false)?;
    let (w, h) = (args.width, args.height);
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("agents-offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = color.create_view(&Default::default());
    let targets = Targets::new(&ctx.device, w, h, format, args.msaa);
    let g = Gpu { ctx, renderer, color, view, targets };
    let aspect = w as f32 / h as f32;
    let heading = folsom_heading(&world);
    let start = Vec3::new(crate::bench::spawn()[0], 0.0, crate::bench::spawn()[1]); // spawn()
    let base_yaw = yaw_of(heading);
    let frames: usize = arg("--ab-frames").and_then(|v| v.parse().ok()).unwrap_or(400);
    let total = WARMUP + frames;
    let path: Vec<Camera> = (0..total)
        .map(|i| {
            let p = start + heading * WALK_M_PER_FRAME * i as f32;
            let y = surface(&world, p.x, p.z) + EYE_M;
            let yaw = base_yaw + 60f32.to_radians() * (std::f32::consts::TAU * i as f32 / total as f32).sin();
            Camera { far: args.far, ..Camera::new(Vec3::new(p.x, y, p.z), yaw, -0.03, aspect) }
        })
        .collect();
    let mut models = ModelRuntime::start(&mut world, args, bases, path[0].eye, process_start);
    let focus = |c: &Camera| AgentRuntime::focus_of(c.eye, c.forward());
    let mut agents = AgentRuntime::start(&world, args, focus(&path[0]));
    crate::props::global_start(&world, args); // props
    settle(&g, &world, &mut models, &path[0])?;
    if !agents.wait_ready(&g.ctx, &g.renderer, focus(&path[0]), 300.0) {
        return Err(anyhow!("agents not ready: {:?}", agents.errors));
    }
    let lib = agents.lib.clone().unwrap();
    println!(
        "agents: library {} ({:.0} ms after start, {} models, {:.2} M triangles in all LODs), simulation ready {:.0} ms after start; renderer VRAM for agents {:.1} MB; missing: {:?}",
        if lib.from_cache { "from cache" } else { "cooked" },
        agents.lib_ready_ms.unwrap_or(f64::NAN),
        lib.models.len(),
        lib.indices.len() as f64 / 3e6,
        agents.sim_ready_ms.unwrap_or(f64::NAN),
        g.renderer.agents.get().map(|a| a.bytes as f64 / 1e6).unwrap_or(0.0),
        lib.meta.missing
    );
    let dir = args.out_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"));
    std::fs::create_dir_all(&dir)?;
    // matched captures and the people pass cost at four poses.
    if std::env::args().any(|a| a == "--people-capture") {
        return crate::people_capture::run(&PeopleGpu { ctx: &g.ctx, renderer: &g.renderer, color: &g.color, view: &g.view, targets: &g.targets }, &mut world, args, &mut models, &mut agents, &path, &dir);
    }
    if std::env::args().any(|a| a == "--agents-capture") {
        return captures(&g, &mut world, args, &mut models, &mut agents, &path, &dir);
    }

    let rounds: usize = arg("--ab-rounds").and_then(|v| v.parse().ok()).unwrap_or(3);
    let densities: Vec<u32> = arg("--densities").map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or(vec![1, 4, 16]);
    let period_ns = g.ctx.queue.get_timestamp_period() as f64;
    let mut runs = vec![];
    for (di, &density) in densities.iter().enumerate() {
        if density != agents.opts.density {
            agents.restart_sim(&world, focus(&path[0]), density);
            agents.opts.on = true;
            if !agents.wait_ready(&g.ctx, &g.renderer, focus(&path[0]), 300.0) {
                return Err(anyhow!("agents not ready at {density}x: {:?}", agents.errors));
            }
        }
        for r in 0..rounds {
            let order = if (r + di) % 2 == 0 { [false, true] } else { [true, false] };
            for on in order {
                let run = measure(&g, &mut agents, &path, on, period_ns)?;
                println!(
                    "  {density:>2}x round {r} agents {:<3}: frame total median {:.3} p95 {:.3} max {:.3} | GPU city pass {:.3} | agent cull {:.3} casters {:.3} | CPU {:.3} (agent pump {:.3}) | agents drawn {:.0}, triangles {:.0} k, shadow triangles {:.0} k",
                    if on { "ON" } else { "OFF" },
                    percentile(&run.total, 50.0),
                    percentile(&run.total, 95.0),
                    percentile(&run.total, 100.0),
                    percentile(&run.city, 50.0),
                    percentile(&run.ag_cull, 50.0),
                    percentile(&run.ag_cast, 50.0),
                    percentile(&run.cpu, 50.0),
                    percentile(&run.ag_cpu, 50.0),
                    percentile(&run.drawn, 50.0),
                    percentile(&run.tris, 50.0) / 1e3,
                    percentile(&run.shadow_tris, 50.0) / 1e3,
                );
                runs.push((density, r, on, run));
            }
        }
    }
    // Pair summary: per density, the median over rounds of each run's median, and ON - OFF.
    println!("\nA/B (median of the per-run medians over {rounds} rounds; ms):");
    println!("density | OFF total med / p95 / MAX | ON total med / p95 / MAX | diff med / p95 | GPU city OFF->ON | agent cull | agent casters | shadow depth OFF->ON | CPU OFF->ON | agents in view / submitted");
    let mut table = vec![];
    for &density in &densities {
        let pick = |on: bool| runs.iter().filter(|(d, _, o, _)| *d == density && *o == on).map(|(_, _, _, r)| r).collect::<Vec<_>>();
        let (off, on) = (pick(false), pick(true));
        let med = |v: &Vec<&Run>, f: &dyn Fn(&Run) -> f64| percentile(&v.iter().map(|r| f(r)).collect::<Vec<_>>(), 50.0);
        let maxof = |v: &Vec<&Run>| v.iter().map(|r| percentile(&r.total, 100.0)).fold(0.0, f64::max);
        let row = json!({
            "density": density,
            "off": { "total_median": med(&off, &|r| percentile(&r.total, 50.0)), "total_p95": med(&off, &|r| percentile(&r.total, 95.0)), "total_max": maxof(&off),
                     "gpu_city": med(&off, &|r| percentile(&r.city, 50.0)), "gpu_shadow_depth": med(&off, &|r| percentile(&r.sh_depth, 50.0)), "gpu_total": med(&off, &|r| percentile(&r.gpu, 50.0)), "cpu": med(&off, &|r| percentile(&r.cpu, 50.0)) },
            "on": { "total_median": med(&on, &|r| percentile(&r.total, 50.0)), "total_p95": med(&on, &|r| percentile(&r.total, 95.0)), "total_max": maxof(&on),
                    "gpu_city": med(&on, &|r| percentile(&r.city, 50.0)), "gpu_shadow_depth": med(&on, &|r| percentile(&r.sh_depth, 50.0)), "gpu_total": med(&on, &|r| percentile(&r.gpu, 50.0)), "cpu": med(&on, &|r| percentile(&r.cpu, 50.0)),
                    "agent_cull": med(&on, &|r| percentile(&r.ag_cull, 50.0)), "agent_cull_p95": med(&on, &|r| percentile(&r.ag_cull, 95.0)), "agent_casters": med(&on, &|r| percentile(&r.ag_cast, 50.0)), "agent_casters_p95": med(&on, &|r| percentile(&r.ag_cast, 95.0)),
                    "agent_cpu_pump": med(&on, &|r| percentile(&r.ag_cpu, 50.0)), "feed_ms": med(&on, &|r| percentile(&r.feed, 50.0)),
                    "agents_drawn": med(&on, &|r| percentile(&r.drawn, 50.0)), "agents_submitted": med(&on, &|r| percentile(&r.submitted, 50.0)), "triangles": med(&on, &|r| percentile(&r.tris, 50.0)), "shadow_triangles": med(&on, &|r| percentile(&r.shadow_tris, 50.0)) },
        });
        let o = &row["off"];
        let n = &row["on"];
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(f64::NAN);
        println!(
            "{density:>6}x | {:.3} / {:.3} / {:.3} | {:.3} / {:.3} / {:.3} | {:+.3} / {:+.3} | {:.3} -> {:.3} | {:.3} | {:.3} | {:.3} -> {:.3} | {:.3} -> {:.3} | {:.0} / {:.0}",
            f(&o["total_median"]), f(&o["total_p95"]), f(&o["total_max"]),
            f(&n["total_median"]), f(&n["total_p95"]), f(&n["total_max"]),
            f(&n["total_median"]) - f(&o["total_median"]), f(&n["total_p95"]) - f(&o["total_p95"]),
            f(&o["gpu_city"]), f(&n["gpu_city"]), f(&n["agent_cull"]), f(&n["agent_casters"]),
            f(&o["gpu_shadow_depth"]), f(&n["gpu_shadow_depth"]), f(&o["cpu"]), f(&n["cpu"]),
            f(&n["agents_drawn"]), f(&n["agents_submitted"]),
        );
        table.push(row);
    }
    let alloc = g.ctx.device.generate_allocator_report();
    let out = json!({
        "path": "eye 1.7 m over the drawn street at the spawn on Folsom Street, 1.4 m/s along it, view swinging +-60 degrees",
        "frames_per_run": frames, "warmup": WARMUP, "rounds": rounds, "width": w, "height": h,
        "frame_total_definition": "CPU (agent pump + update + encode + submit) + GPU first-pass-begin..post-end, summed per frame",
        "agents_vram_bytes": g.renderer.agents.get().map(|a| a.bytes),
        "renderer_vram_bytes": g.renderer.bytes.total(),
        "wgpu_allocator_total_allocated": alloc.as_ref().map(|a| a.total_allocated_bytes),
        "library": { "from_cache": lib.from_cache, "load_ms": lib.load_ms, "models": lib.models.len(), "meshes": lib.meshes.len(), "vertices": lib.vertices.len(), "triangles": lib.indices.len() / 3, "missing": lib.meta.missing },
        "table": table,
        "runs": runs.iter().map(|(d, r, on, run)| json!({ "density": d, "round": r, "on": on, "total": summary(&run.total), "gpu": summary(&run.gpu), "city": summary(&run.city), "agent_cull": summary(&run.ag_cull), "agent_casters": summary(&run.ag_cast), "shadow_depth": summary(&run.sh_depth), "cpu": summary(&run.cpu), "agent_cpu": summary(&run.ag_cpu), "drawn": summary(&run.drawn) })).collect::<Vec<_>>(),
    });
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let p = dir.join(format!("agents-ab-{}-{stamp}.json", args.label.as_deref().unwrap_or("default")));
    std::fs::write(&p, serde_json::to_string_pretty(&out)?)?;
    println!("wrote {}", p.display());
    Ok(())
}

#[derive(Default)]
pub struct Run {
    pub total: Vec<f64>,
    pub gpu: Vec<f64>,
    pub cpu: Vec<f64>,
    pub ag_cpu: Vec<f64>,
    pub city: Vec<f64>,
    pub sh_depth: Vec<f64>,
    pub ag_cull: Vec<f64>,
    pub ag_cast: Vec<f64>,
    pub drawn: Vec<f64>,
    pub submitted: Vec<f64>,
    pub tris: Vec<f64>,
    pub shadow_tris: Vec<f64>,
    pub feed: Vec<f64>,
}

fn measure(g: &Gpu, agents: &mut AgentRuntime, path: &[Camera], on: bool, period_ns: f64) -> Result<Run> {
    measure_dt(&PeopleGpu { ctx: &g.ctx, renderer: &g.renderer, color: &g.color, view: &g.view, targets: &g.targets }, agents, path, on, period_ns, FRAME_S)
}

/// the borrowed GPU objects the people captures use.
pub struct PeopleGpu<'a> {
    pub ctx: &'a GpuContext,
    pub renderer: &'a Renderer,
    pub color: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
    pub targets: &'a Targets,
}

impl PeopleGpu<'_> {
    pub fn attach(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match &self.targets.msaa {
            Some(m) => (m, Some(self.view)),
            None => (self.view, None),
        }
    }
}

/// `measure` with the frame step `dt` (0 holds the simulation still).
pub fn measure_dt(g: &PeopleGpu, agents: &mut AgentRuntime, path: &[Camera], on: bool, period_ns: f64, dt: f64) -> Result<Run> {
    let ctx = &g.ctx;
    let total = path.len();
    let ts_frames = (4096 / QUERIES_PER_FRAME) as usize;
    let sets: Vec<wgpu::QuerySet> = (0..total.div_ceil(ts_frames))
        .map(|_| ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("ab-ts"), ty: wgpu::QueryType::Timestamp, count: QUERIES_PER_FRAME * ts_frames as u32 }))
        .collect();
    let aq = eo_render::agents::QUERIES_PER_FRAME;
    let agent_set = ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("ab-agent-ts"), ty: wgpu::QueryType::Timestamp, count: aq * total as u32 });
    let sw = eo_render::agents::STATS_WORDS as u64 * 4;
    let stats = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ab-stats"), size: sw * total as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let ag = g.renderer.agents.get();
    if let Some(a) = ag {
        a.set_enabled(on);
    }
    agents.opts.on = on;
    let mut run = Run::default();
    let mut cpu = vec![];
    let mut ag_cpu = vec![];
    let mut subs: Vec<wgpu::SubmissionIndex> = vec![];
    let (attach, resolve) = g.attach();
    for (i, cam) in path.iter().enumerate() {
        if i >= 2 {
            ctx.device.poll(wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }).map_err(|e| anyhow!("{e}"))?;
        }
        let t0 = Instant::now();
        if on {
            let s = agents.pump(ctx, g.renderer, AgentRuntime::focus_of(cam.eye, cam.forward()), dt);
            ag_cpu.push(s.cpu_ms);
            run.feed.push(s.feed_ms);
        }
        g.renderer.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        let ts = sets.get(i / ts_frames).map(|q| (q, QUERIES_PER_FRAME * (i % ts_frames) as u32));
        if let Some(a) = ag {
            a.set_timestamps(on.then(|| (agent_set.clone(), aq * i as u32)));
        }
        g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, ts);
        if let Some(a) = ag {
            if on {
                enc.copy_buffer_to_buffer(a.stats_buffer(), 0, &stats, i as u64 * sw, sw);
            }
        }
        subs.push(ctx.queue.submit([enc.finish()]));
        cpu.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    if let Some(a) = ag {
        a.set_timestamps(None);
    }
    let read_ts = |q: &wgpu::QuerySet, n: u32| -> Result<Vec<u64>> {
        let rb = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ts-resolve"), size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let mb = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ts-read"), size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.resolve_query_set(q, 0..n, &rb, 0);
        enc.copy_buffer_to_buffer(&rb, 0, &mb, 0, n as u64 * 8);
        ctx.queue.submit([enc.finish()]);
        Ok(read_mapped(ctx, &mb)?.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
    };
    let mut ts = vec![];
    for q in &sets {
        ts.extend(read_ts(q, QUERIES_PER_FRAME * ts_frames as u32)?);
    }
    let ats = if on { read_ts(&agent_set, aq * total as u32)? } else { vec![] };
    let st: Vec<u32> = if on { bytemuck::pod_collect_to_vec(&read_mapped(ctx, &stats)?) } else { vec![] };
    let ms = |a: u64, b: u64| b.wrapping_sub(a) as f64 * period_ns / 1e6;
    for i in WARMUP..total {
        let b = i * QUERIES_PER_FRAME as usize;
        // post is the last pair; the city is the early + the late pass.
        let (qp, ql) = (eo_render::look::Q_POST as usize, eo_render::Q_LATE_DRAW as usize);
        let gpu = ms(ts[b], ts[b + qp + 1]);
        run.gpu.push(gpu);
        run.city.push(ms(ts[b + 2], ts[b + 3]) + ms(ts[b + ql], ts[b + ql + 1]));
        run.sh_depth.push(ms(ts[b + 6], ts[b + 7]));
        run.cpu.push(cpu[i]);
        run.total.push(cpu[i] + gpu);
        if on {
            let a = i * aq as usize;
            run.ag_cull.push(ms(ats[a], ats[a + 1]));
            run.ag_cast.push(ms(ats[a + 2], ats[a + 3]));
            run.ag_cpu.push(ag_cpu[i]);
            let s = &st[i * eo_render::agents::STATS_WORDS..];
            run.drawn.push(s[0] as f64);
            run.tris.push(s[1] as f64);
            run.shadow_tris.push((s[3] + s[5] + s[7]) as f64);
            run.submitted.push((s[8] + s[9]) as f64);
        }
    }
    Ok(run)
}

/// Render one camera into the offscreen target and save it as PNG (as bench.rs's capture).
fn capture(g: &Gpu, cam: &Camera, path: &std::path::Path) -> Result<()> {
    let ctx = &g.ctx;
    let (w, h) = (g.color.width(), g.color.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let (attach, resolve) = g.attach();
    // Several frames: LOD hysteresis and every shadow cascade refreshed for this camera.
    for _ in 0..6 {
        g.renderer.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
        ctx.queue.submit([enc.finish()]);
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

pub(crate) fn look_at(eye: Vec3, at: Vec3, far: f32, aspect: f32, fov: f32) -> Camera {
    let d = at - eye;
    let pitch = (d.y / Vec3::new(d.x, 0.0, d.z).length().max(1e-3)).atan();
    Camera { far, fov_y_deg: fov, ..Camera::new(eye, yaw_of(d), pitch, aspect) }
}

/// A camera `dist` from `at` with a clear view: the first of 24 bearings
/// (starting at `bearing`) whose eye and sight line stay out of buildings.
pub(crate) fn clear_view(world: &World, at: Vec3, bearing: f32, dist: f32, eye_h: f32, aim_h: f32, far: f32, aspect: f32, fov: f32) -> Camera {
    for k in 0..24 {
        let a = bearing + (k as f32 * 0.5).ceil() * if k % 2 == 0 { 1.0 } else { -1.0 } * std::f32::consts::TAU / 24.0;
        let e = at + Vec3::new(a.sin() * dist, 0.0, a.cos() * dist);
        // The sight line and 4 m behind the eye stay out of every footprint.
        let behind = e + (e - at).normalize_or_zero() * 4.0;
        let blocked = (0..=12).any(|s| {
            let p = at.lerp(behind, s as f32 / 12.0);
            inside_building(world, p.x, p.z)
        });
        if !blocked {
            let eye = Vec3::new(e.x, surface(world, e.x, e.z) + eye_h, e.z);
            return look_at(eye, at + Vec3::Y * aim_h, far, aspect, fov);
        }
    }
    let e = at + Vec3::new(bearing.sin() * dist, 0.0, bearing.cos() * dist);
    look_at(Vec3::new(e.x, surface(world, e.x, e.z) + eye_h, e.z), at + Vec3::Y * aim_h, far, aspect, fov)
}

pub(crate) fn heading_of(a: &GpuAgent) -> f32 {
    let f = glam::Quat::from_array(a.rot) * Vec3::Z;
    f.x.atan2(f.z)
}

fn captures(g: &Gpu, world: &mut World, args: &Args, models: &mut ModelRuntime, agents: &mut AgentRuntime, path: &[Camera], dir: &std::path::Path) -> Result<()> {
    let aspect = g.color.width() as f32 / g.color.height() as f32;
    let far = args.far;
    let lib = agents.lib.clone().unwrap();
    let is_person = |a: &GpuAgent| lib.models.get(a.model as usize).is_some_and(|m| m.class == CLASS_PERSON);
    // Let the street run a little, then freeze the simulation (dt 0) for the shots.
    let cam0 = path[0];
    // Run until someone is on a crosswalk near the spawn (at most 60 s of simulation).
    // `EO_GS_CAPTURE=1` runs exactly 600 steps (10 s) instead, the same for every variant.
    let gs_capture = std::env::var("EO_GS_CAPTURE").is_ok();
    for step in 0..if gs_capture { 600 } else { 3600 } {
        if !gs_capture && step >= 240 && step % 30 == 0 {
            let snap = agents.snapshot();
            if snap.iter().any(|a| is_person(a) && a.flags & eo_sim::output::FLAG_CROSSING as u32 != 0 && Vec3::from(a.pos).distance(cam0.eye) < 90.0) {
                break;
            }
        }
        agents.pump(&g.ctx, &g.renderer, AgentRuntime::focus_of(cam0.eye, cam0.forward()), FRAME_S);
        g.renderer.update(&g.ctx.queue, &cam0);
        let mut enc = g.ctx.device.create_command_encoder(&Default::default());
        let (attach, resolve) = g.attach();
        g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
        g.ctx.queue.submit([enc.finish()]);
        let _ = g.ctx.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let freeze = |agents: &mut AgentRuntime, cam: &Camera| {
        for _ in 0..3 {
            agents.pump(&g.ctx, &g.renderer, AgentRuntime::focus_of(cam.eye, cam.forward()), 0.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    freeze(agents, &cam0);
    let list: Vec<GpuAgent> = agents.snapshot();
    let cars: Vec<&GpuAgent> = list.iter().filter(|a| !is_person(a)).collect();
    let people: Vec<&GpuAgent> = list.iter().filter(|a| is_person(a)).collect();
    println!(
        "capture: {} cars and {} people around the spawn; {} walkers moved from a drawn carriageway onto its footway",
        cars.len(),
        people.len(),
        agents.moved_to_footway()
    );
    let p = |a: &GpuAgent| Vec3::from(a.pos);
    let near = |v: &[&GpuAgent], at: Vec3| v.iter().min_by(|a, b| p(a).distance(at).total_cmp(&p(b).distance(at))).map(|a| **a);
    let mut shots: Vec<(String, Camera, String)> = vec![];
    // `EO_GS_CAPTURE=1`: the street shot from a fixed placement (not aimed at
    // the cars): 2.2 m over the walk's start, looking at the Folsom Street centreline 40 m
    // (times the ground scale) ahead, 1 m over the road; the same spot at every scale.
    if gs_capture {
        let heading = folsom_heading(world);
        let gs = eo_data::groundscale::scale() as f32;
        let e2 = glam::Vec2::new(cam0.eye.x, cam0.eye.z);
        let mut best = (f32::MAX, e2);
        for r in world.city.roads.iter().filter(|r| r.name == "Folsom Street") {
            for w in r.points.windows(2) {
                let (a, b) = (glam::Vec2::from(w[0]), glam::Vec2::from(w[1]));
                let ab = b - a;
                let t = ((e2 - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let q = a + ab * t;
                if q.distance(e2) < best.0 {
                    best = (q.distance(e2), q);
                }
            }
        }
        let c = best.1 + glam::Vec2::new(heading.x, heading.z) * 40.0 * gs;
        let eye = Vec3::new(cam0.eye.x, surface(world, cam0.eye.x, cam0.eye.z) + 2.2, cam0.eye.z);
        let at = Vec3::new(c.x, surface(world, c.x, c.y) + 1.0, c.y);
        shots.push(("street".into(), look_at(eye, at, far, aspect, eo_render::camera::GAME_FOV_Y_DEG), format!("fixed: {:.1} m from the Folsom centreline, {} cars within 100 m", best.0, cars.iter().filter(|a| p(a).distance(eye) < 100.0).count())));
    }
    // 1. The street with traffic: from the walk, looking at the middle of the nearest cars.
    if !gs_capture && !cars.is_empty() {
        let mut by: Vec<&&GpuAgent> = cars.iter().collect();
        by.sort_by(|a, b| p(a).distance(cam0.eye).total_cmp(&p(b).distance(cam0.eye)));
        let k = by.len().min(6);
        let c = by[..k].iter().map(|a| p(a)).sum::<Vec3>() / k as f32;
        let eye = Vec3::new(cam0.eye.x, surface(world, cam0.eye.x, cam0.eye.z) + 2.2, cam0.eye.z);
        let mut cam = look_at(eye, c + Vec3::Y * 1.0, far, aspect, eo_render::camera::GAME_FOV_Y_DEG);
        if c.distance(eye) < 15.0 {
            cam = clear_view(world, c, heading_of(by[0]) + 2.6, 28.0, 2.5, 1.0, far, aspect, eo_render::camera::GAME_FOV_Y_DEG);
        }
        shots.push(("street".into(), cam, format!("{} cars within 100 m", cars.iter().filter(|a| p(a).distance(c) < 100.0).count())));
    }
    // 2. A crossing: people waiting at the kerb or on a crosswalk.
    use eo_sim::output::{FLAG_CROSSING, FLAG_KERB_WAIT};
    let on_crosswalk: Vec<&GpuAgent> = people.iter().copied().filter(|a| a.flags & FLAG_CROSSING as u32 != 0).collect();
    let at_kerb: Vec<&GpuAgent> = people.iter().copied().filter(|a| a.flags & FLAG_KERB_WAIT as u32 != 0).collect();
    let crowd_at = if let Some(a) = near(&on_crosswalk, cam0.eye) {
        Some((a, format!("{} people on a crosswalk, {} waiting at a kerb", on_crosswalk.len(), at_kerb.len())))
    } else if let Some(a) = near(&at_kerb, cam0.eye) {
        Some((a, format!("nobody on a crosswalk; {} waiting at a kerb", at_kerb.len())))
    } else {
        near(&people, cam0.eye).map(|a| (a, "nearest people (none at a crossing)".to_string()))
    };
    if let Some((a, what)) = crowd_at {
        let at = p(&a);
        shots.push(("crossing".into(), clear_view(world, at, heading_of(&a) + 1.6, 15.0, 3.5, 1.0, far, aspect, eo_render::camera::GAME_FOV_Y_DEG), what));
    }
    // 3. A close person (moving if any).
    let walking: Vec<&GpuAgent> = people.iter().copied().filter(|a| a.speed > 0.6).collect();
    if let Some(a) = near(&walking, cam0.eye).or_else(|| near(&people, cam0.eye)) {
        shots.push(("person".into(), clear_view(world, p(&a), heading_of(&a) + 0.6, 3.2, 1.55, 1.05, far, aspect, 40.0), format!("speed {:.2} m/s, clip {}", a.speed, a.clip)));
    }
    // 4. A close car.
    if let Some(a) = near(&cars, cam0.eye) {
        shots.push(("car".into(), clear_view(world, p(&a), heading_of(&a) + 0.8, 7.0, 1.5, 0.8, far, aspect, 45.0), format!("{} at {:.1} m/s, braking {}", lib.meta.model_names[a.model as usize], a.speed, a.flags & 1)));
    }
    for (name, cam, what) in &shots {
        settle(g, world, models, cam)?;
        agents.parked = crate::props::global_parked(agents.lib.as_deref(), cam.eye); // props
        freeze(agents, cam);
        capture(g, cam, &dir.join(format!("capture-agents-{name}.png")))?;
        println!("  {name}: eye {:.1} {:.2} {:.1}: {what}", cam.eye.x, cam.eye.y, cam.eye.z);
    }
    player_captures(g, world, args, models, agents, dir)
}

/// how far the matched captures walk and drive before the shot, metres at
/// ground scale 1 (times the scale otherwise).
const WALK_CAPTURE_M: f64 = 16.0;
const DRIVE_CAPTURE_M: f64 = 25.0;

/// distance along a polyline route to the projection of `p` on it.
fn route_along(route: &[[f64; 2]], p: [f64; 2]) -> f64 {
    let (mut best, mut acc, mut at) = (f64::MAX, 0.0, 0.0);
    for w in route.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz);
        if l < 1e-9 {
            continue;
        }
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / (l * l)).clamp(0.0, 1.0);
        let d = (a[0] + dx * t - p[0]).hypot(a[1] + dz * t - p[1]);
        if d < best {
            best = d;
            at = acc + l * t;
        }
        acc += l;
    }
    at
}

/// a walking route on the drawn street plan: the Folsom Street run nearest
/// `p`, from `p`'s station along the run (in the `heading` direction), down the middle of
/// the footway on `p`'s side; points every 5 m. None without the street world.
fn folsom_footway_route(world: &World, p: [f64; 2], heading: Vec3) -> Option<Vec<[f64; 2]>> {
    let sw = &world.street.as_ref()?.world;
    let mut best: Option<(f64, usize, f64)> = None;
    for (ei, e) in sw.plan.entries.iter().enumerate() {
        if sw.city.segments[e.road].street_name != "Folsom Street" || e.points.len() < 2 || e.cum.len() != e.points.len() {
            continue;
        }
        for i in 0..e.points.len() - 1 {
            let (a, b) = (e.points[i], e.points[i + 1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dz * dz;
            if l2 < 1e-9 {
                continue;
            }
            let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0);
            let d = (a[0] + dx * t - p[0]).hypot(a[1] + dz * t - p[1]);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, ei, e.cum[i] + (e.cum[i + 1] - e.cum[i]) * t));
            }
        }
    }
    let (_, ei, st) = best?;
    let e = &sw.plan.entries[ei];
    let f = e.frame(st, false);
    let side = if (p[0] - f.x) * f.nx + (p[1] - f.z) * f.nz >= 0.0 { 1.0 } else { -1.0 };
    let walk = e.walks[if side > 0.0 { 0 } else { 1 }];
    let u = side * (e.half + walk * 0.5);
    let dir = if f.tx * heading.x as f64 + f.tz * heading.z as f64 >= 0.0 { 1.0 } else { -1.0 };
    let mut out = vec![];
    for k in 0..40 {
        let s = st + dir * 5.0 * k as f64;
        if s < 0.0 || s > e.length {
            break;
        }
        let q = e.frame(s, false);
        out.push([q.x + q.nx * u, q.z + q.nz * u]);
    }
    (out.len() >= 2).then_some(out)
}

/// The player on foot (third person) and in the player's car, as `--play` draws them.
fn player_captures(g: &Gpu, world: &mut World, args: &Args, models: &mut ModelRuntime, agents: &mut AgentRuntime, dir: &std::path::Path) -> Result<()> {
    use eo_player::routes::{DrivePilot, WalkPilot};
    use eo_player::street_ground::StreetGround;
    use eo_player::world::{BuildingSet, CollisionWorld, ElevationGround, GroundProvider};
    use eo_player::{Mode, Player, StepInput};
    use std::sync::Arc;
    let aspect = g.color.width() as f32 / g.color.height() as f32;
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let buildings = Arc::new(BuildingSet::from_city(&world.city, lift));
    let ground: Arc<dyn GroundProvider> = match &world.street {
        Some(s) => Arc::new(StreetGround::new(s.world.clone(), world.structures.clone())),
        None => Arc::new(ElevationGround(Arc::new(world.elevation.clone()))),
    };
    let mut cw = CollisionWorld::new(ground, buildings, 2);
    // Folsom Street at the spawn, along it.
    let heading = folsom_heading(world);
    let s = crate::bench::spawn(); // spawn()
    let gs_capture = std::env::var("EO_GS_CAPTURE").is_ok();
    let gs = eo_data::groundscale::scale();
    let mut route: Vec<[f64; 2]> = (0..40).map(|k| [s[0] as f64 + heading.x as f64 * 10.0 * k as f64, s[1] as f64 + heading.z as f64 * 10.0 * k as f64]).collect();
    // `EO_GS_CAPTURE=1`: the walk anchored to the drawn street plan, not to a
    // world point: the middle of the footway on the spawn's side of the Folsom Street run
    // nearest the spawn, from the spawn's station along the street (the same block face
    // and place across the street at every ground scale).
    if gs_capture {
        if let Some(r) = folsom_footway_route(world, [s[0] as f64, s[1] as f64], heading) {
            route = r;
        }
    }
    let mut start = DVec3::new(route[0][0], world.elevation.ground(route[0][0], route[0][1]), route[0][1]);
    cw.load_blocking(start, eo_player::world::LOAD_RADIUS_M);
    start.y = cw.surface_y(start.x, start.z);
    let yaw0 = (route[1][0] - route[0][0]).atan2(route[1][1] - route[0][1]);
    let mut player = Player::new(start, yaw0);
    let mut visual = PlayerVisual::default();
    let dt = eo_player::tuning::FIXED_STEP_S;
    let to_camera = |v: &eo_player::CameraView| {
        let f = v.forward.normalize_or(DVec3::Z);
        Camera { fov_y_deg: v.fov_deg as f32, far: args.far, ..Camera::new(Vec3::new(v.eye.x as f32, v.eye.y as f32, v.eye.z as f32), f.x.atan2(-f.z) as f32, f.y.clamp(-1.0, 1.0).asin() as f32, aspect) }
    };
    let mut pilot = WalkPilot::new(route.clone(), true);
    let mut view = player.camera(&cw, dt, aspect as f64);
    let frame = |agents: &mut AgentRuntime, player: &Player, cw: &CollisionWorld, view: &eo_player::CameraView, visual: &mut PlayerVisual, dt: f64| {
        let cam = to_camera(view);
        agents.set_player(visual, player, cw, view.body_visible, dt);
        let pos = player.position(cw);
        agents.pump(&g.ctx, &g.renderer, [pos.x, pos.z, player.yaw], dt);
        cam
    };
    // `EO_GS_CAPTURE=1` walks until 16 m (times the ground scale) along the
    // route instead of 150 steps, so the pose lands at the same station of the block.
    let walk_to = WALK_CAPTURE_M * gs;
    for step in 0..if gs_capture { 900 } else { 150 } {
        if gs_capture && step > 0 {
            let p = player.position(&cw);
            let d = route_along(&route, [p.x, p.z]);
            if d >= walk_to {
                break;
            }
        }
        let input = pilot.input(&mut player, &cw);
        player.advance(&mut cw, &input, dt);
        cw.pump(player.position(&cw));
        view = player.camera(&cw, dt, aspect as f64);
        frame(agents, &player, &cw, &view, &mut visual, dt);
    }
    // `EO_GS_CAPTURE=1`: no simulated walker whose body comes within 1.5 m of the lens
    // (one passing between the lens and the player covers the frame at random).
    if gs_capture {
        agents.hide_near = vec![([view.eye.x as f32, view.eye.y as f32, view.eye.z as f32], 1.5)];
    }
    let cam = frame(agents, &player, &cw, &view, &mut visual, 0.0);
    settle(g, world, models, &cam)?;
    capture(g, &cam, &dir.join("capture-agents-play-walk.png"))?;
    agents.hide_near.clear();
    println!("  play-walk: body visible {}, speed {:.2} m/s, walk phase {:.2}", view.body_visible, player.ground_speed, visual.phase);
    // The car: spawn beside the walker, get in, drive along the street.
    let f = player.character.feet();
    // On Folsom Street's centreline (the extract's way) nearest the walker, along the street.
    let mut best = (f64::MAX, [f.x, f.z]);
    for r in world.city.roads.iter().filter(|r| r.name == "Folsom Street") {
        for w in r.points.windows(2) {
            let (a, b) = ([w[0][0] as f64, w[0][1] as f64], [w[1][0] as f64, w[1][1] as f64]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let t = (((f.x - a[0]) * dx + (f.z - a[1]) * dz) / (dx * dx + dz * dz).max(1e-9)).clamp(0.0, 1.0);
            let p = [a[0] + dx * t, a[1] + dz * t];
            let d = (p[0] - f.x).hypot(p[1] - f.z);
            if d < best.0 {
                best = (d, p);
            }
        }
    }
    let side = (0.0, best.1[0], best.1[1]);
    let k = player.spawn_vehicle(&mut cw, side.1, side.2, player.facing_yaw);
    // Step up to the driver's side, as `--bench --play` does before the toggle.
    let to = DVec3::new(f.x - side.1, 0.0, f.z - side.2).normalize_or(DVec3::X);
    let door = DVec3::new(side.1 + to.x * 1.6, 0.0, side.2 + to.z * 1.6);
    player.character.teleport_feet(DVec3::new(door.x, cw.surface_y(door.x, door.z), door.z));
    for _ in 0..30 {
        player.fixed_step(&mut cw, &StepInput::default());
    }
    player.fixed_step(&mut cw, &StepInput { vehicle_toggle: true, ..Default::default() });
    if !matches!(player.mode, Mode::Driving(_)) {
        eprintln!("  play-drive: could not enter the car (vehicle {k})");
    }
    let p0 = player.position(&cw);
    let drive_route: Vec<[f64; 2]> = (0..40).map(|j| [p0.x + heading.x as f64 * 10.0 * j as f64, p0.z + heading.z as f64 * 10.0 * j as f64]).collect();
    let mut dp = DrivePilot::new(drive_route.clone(), 30.0 / 3.6);
    // `EO_GS_CAPTURE=1` drives until 25 m (times the ground scale) along the
    // street from the car's start instead of 240 steps (the same station at every scale).
    let drive_to = DRIVE_CAPTURE_M * gs;
    for step in 0..if gs_capture { 1200 } else { 240 } {
        if gs_capture && step > 0 {
            let p = player.position(&cw);
            if route_along(&drive_route, [p.x, p.z]) >= drive_to {
                break;
            }
        }
        let input = dp.input(&player, &cw);
        player.advance(&mut cw, &input, dt);
        cw.pump(player.position(&cw));
        view = player.camera(&cw, dt, aspect as f64);
        frame(agents, &player, &cw, &view, &mut visual, dt);
    }
    // `EO_GS_CAPTURE=1`: no simulated car drawn through the player's car or
    // the lens (the simulation does not see the player's car), the same for every variant.
    if gs_capture {
        let pc = player.position(&cw);
        agents.hide_near = vec![([pc.x as f32, pc.y as f32, pc.z as f32], 2.5), ([view.eye.x as f32, view.eye.y as f32, view.eye.z as f32], 1.0)];
    }
    let cam = frame(agents, &player, &cw, &view, &mut visual, 0.0);
    settle(g, world, models, &cam)?;
    capture(g, &cam, &dir.join("capture-agents-play-drive.png"))?;
    agents.hide_near.clear();
    let (mode, speed) = match player.mode {
        Mode::Driving(i) => ("driving", player.vehicles[i].forward_speed(&cw)),
        Mode::OnFoot => ("on foot", 0.0),
    };
    println!("  play-drive: {mode}, speed {speed:.1} m/s");
    Ok(())
}
