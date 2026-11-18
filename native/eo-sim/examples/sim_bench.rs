//! The simulation's cost per step at 1x, 4x and 16x the game's default
//! population, with PASS/FAIL against the 1 ms main-thread budget at 1x.
//!
//!   cargo run --release -p eo-sim --example sim_bench
//!   cargo run --release -p eo-sim --example sim_bench -- --steps 3600 --warmup 600 --threads 8
//!
//! DEFAULT POPULATION (1x): one region around one player with the shipped
//! defaults - 42 moving cars (`STREET_POPULATION.vehiclePool`, the High
//! vehicle preset's `trafficMax`) and the People `medium` disc
//! (`DEFAULT_PEOPLE_LEVEL`: 40 bodies within 55 m), car recycler and level of
//! detail on. Nx is N such regions (N players, or N interest points of a
//! server), each around a different signalised junction of San Francisco,
//! on a network built natively from the extract for the tiles around them.
//! Each focus walks a 40 m circle at walking pace and turns its view, so the
//! recycler, the disc's scan and retire, and the level of detail all work.
//!
//! MODES, per population:
//!   inline   every region stepped on the calling thread: what the step costs
//!            the render thread if it ran there (the budget's worst case);
//!   pool     regions stepped on the worker pool: wall time per step and the
//!            CPU time summed over every thread;
//!   async    the world on its own thread (`AsyncSim`): what the render thread
//!            actually pays per frame (post the frame's inputs, pick up the
//!            newest buffers) at 60 frames a second.
//! Plus `window`: the JS oracle's own configuration (42 cars, the 300-walker
//! ambient pool, no camera) on the shipped window, beside the JS ms/step.

use std::time::Instant;

use eo_sim::native;
use eo_sim::region::{Focus, Region, RegionConfig};
use eo_sim::{AsyncSim, Network, World};

fn arg(name: &str, default: usize) -> usize {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Stat {
    median: f64,
    p95: f64,
    max: f64,
    mean: f64,
}

fn stat(mut v: Vec<f64>) -> Stat {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len().max(1);
    Stat {
        median: v[n / 2],
        p95: v[((n as f64 * 0.95) as usize).min(n - 1)],
        max: v[n - 1],
        mean: v.iter().sum::<f64>() / n as f64,
    }
}

fn focus_at(c: [f64; 2], t: f64) -> Focus {
    let w = 1.4 / 40.0;
    let a = t * w;
    Focus { x: c[0] + 40.0 * a.cos(), z: c[1] + 40.0 * a.sin(), yaw: a + std::f64::consts::FRAC_PI_2 + (t * 0.3).sin() * 0.8 }
}

fn main() -> anyhow::Result<()> {
    let steps = arg("--steps", 3600);
    let warmup = arg("--warmup", 600);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let max_threads = arg("--threads", cores.saturating_sub(2).max(1));
    let paths = eo_data::DataPaths::discover()?;

    let t0 = Instant::now();
    let source = native::load(&paths)?;
    println!("native source: {} vehicle ways, loaded in {:.2} s", source.road_count(), t0.elapsed().as_secs_f64());

    // Foci: signalised junctions spread over the city, at least 900 m apart, deterministic.
    let signals: Vec<[f64; 2]> = eo_data::sim_source::SimSource::load(&paths)?.signals;
    let mut sorted = signals.clone();
    sorted.sort_by(|a, b| (a[0] + a[1] * 0.37).total_cmp(&(b[0] + b[1] * 0.37)));
    let mut foci: Vec<[f64; 2]> = vec![[1596.6, -318.4]]; // downtown SoMa, inside the JS window
    for s in sorted {
        if foci.len() >= 16 {
            break;
        }
        if foci.iter().all(|f| (f[0] - s[0]).hypot(f[1] - s[1]) >= 900.0) && s[0].abs() < 5000.0 && s[1].abs() < 5000.0 {
            foci.push(s);
        }
    }

    println!();
    println!("population | mode   | threads | ms/step median   p95     max | CPU ms/step (all threads) | steps/s | cars  walkers far% | main-thread ms/frame");
    println!("-----------+--------+---------+------------------------------+---------------------------+---------+--------------------+---------------------");
    let mut pass = true;
    for &n in &[1usize, 4, 16] {
        // Network for every focus's tiles (600 m around it).
        let mut tiles = vec![];
        for f in &foci[..n] {
            tiles.extend(native::tiles_around(f[0], f[1], 600.0));
        }
        tiles.sort();
        tiles.dedup();
        let build_net = || {
            let mut nn = Network::new(source.timing.clone());
            nn.add(source.build(&tiles));
            nn
        };
        let t1 = Instant::now();
        let net = build_net();
        let build_s = t1.elapsed().as_secs_f64();
        let lanes = net.alive_lanes().count();
        let footways = net.alive_footways().count();
        let make_world = |net: Network| {
            let mut world = World::new(net);
            for (k, f) in foci[..n].iter().enumerate() {
                let r = Region::new(&world.net, k as u32, RegionConfig::game_default(1), focus_at(*f, 0.0));
                world.regions.push(r);
            }
            world
        };
        println!(
            "{n:>2}x network: {} tiles, {lanes} lanes, {footways} footways, {} signal junctions, built in {:.2} s",
            tiles.len(),
            net.junctions.len(),
            build_s
        );

        for mode in ["inline", "pool", "async"] {
            let mut world = make_world(build_net());
            let threads = if mode == "pool" { max_threads.min(n.saturating_sub(1)) } else { 0 };
            world.set_threads(threads);
            for s in 0..warmup {
                for (k, f) in foci[..n].iter().enumerate() {
                    world.set_focus(k, focus_at(*f, s as f64 / 60.0));
                }
                world.step();
            }
            let mut wall = vec![];
            let mut cpu = vec![];
            let mut main_ms = vec![];
            let (mut cars, mut walkers, mut far, mut agents) = (0usize, 0usize, 0usize, 0usize);
            if mode == "async" {
                let mut sim = AsyncSim::start(world);
                let frame = 1.0 / 60.0;
                let mut foci_buf: Vec<(usize, Focus)> = Vec::with_capacity(n);
                let mut last_steps = 0;
                for s in 0..steps {
                    let t = (warmup + s) as f64 / 60.0;
                    foci_buf.clear();
                    for (k, f) in foci[..n].iter().enumerate() {
                        foci_buf.push((k, focus_at(*f, t)));
                    }
                    let a = Instant::now();
                    sim.post(frame, &foci_buf);
                    let fr = sim.latest();
                    let (c, w) = (fr.regions.iter().map(|r| r.vehicles.len()).sum::<usize>(), fr.regions.iter().map(|r| r.walkers.len()).sum::<usize>());
                    main_ms.push(a.elapsed().as_secs_f64() * 1000.0);
                    if fr.steps != last_steps {
                        last_steps = fr.steps;
                        wall.push(fr.timing.wall_ns as f64 / 1e6);
                        cpu.push(fr.timing.cpu_ns as f64 / 1e6);
                    }
                    cars += c;
                    walkers += w;
                    agents += 1;
                    // Pace the render loop at 60 frames a second.
                    let spent = a.elapsed().as_secs_f64();
                    if spent < frame {
                        std::thread::sleep(std::time::Duration::from_secs_f64(frame - spent));
                    }
                }
                drop(sim.stop());
            } else {
                for s in 0..steps {
                    for (k, f) in foci[..n].iter().enumerate() {
                        world.set_focus(k, focus_at(*f, (warmup + s) as f64 / 60.0));
                    }
                    world.step();
                    wall.push(world.last.wall_ns as f64 / 1e6);
                    cpu.push(world.last.cpu_ns as f64 / 1e6);
                    for r in &world.regions {
                        cars += r.fleet.cars.len();
                        walkers += r.crowd.walkers.len();
                        far += r.fleet.cars.iter().filter(|c| c.far).count() + r.crowd.walkers.iter().filter(|w| w.far).count();
                    }
                    agents += 1;
                }
            }
            let w = stat(wall);
            let c = stat(cpu);
            let per = agents.max(1) as f64;
            let main = if mode == "async" {
                let m = stat(main_ms);
                format!("{:.4} median, {:.4} p95, {:.3} max", m.median, m.p95, m.max)
            } else {
                // Inline at one fixed step per 60 Hz frame: the step IS the main-thread cost.
                format!("{:.4} median, {:.4} p95 (= step)", w.median, w.p95)
            };
            let far_pct = if cars + walkers > 0 { 100.0 * far as f64 / (cars + walkers) as f64 } else { 0.0 };
            println!(
                "{n:>9}x | {mode:<6} | {:>7} | {:>8.4} {:>8.4} {:>7.3} | {:>10.4} mean {:>6.4} p95 | {:>7.0} | {:>5.0} {:>7.0} {:>4.0} | {main}",
                if mode == "async" { 1 + n.min(1) } else { 1 + threads },
                w.median,
                w.p95,
                w.max,
                c.mean,
                c.p95,
                1000.0 / w.mean.max(1e-9),
                cars as f64 / per,
                walkers as f64 / per,
                far_pct
            );
            if n == 1 && mode == "inline" {
                let ok = w.p95 <= 1.0;
                pass &= ok;
                println!("           budget: inline step p95 {:.4} ms <= 1 ms main-thread at 1x: {}", w.p95, if ok { "PASS" } else { "FAIL" });
            }
        }
        drop(net);
    }

    // The JS oracle's configuration on the shipped window.
    let (wnet, oracle) = eo_sim::window_network(&paths)?;
    let mut world = World::new(wnet);
    let cfg = RegionConfig::js_pool(oracle.meta.seed_int, oracle.run.ambient_walkers);
    world.regions.push(Region::new(&world.net, 0, cfg, Focus { x: 1596.6, z: -318.4, yaw: f64::NAN }));
    for _ in 0..warmup {
        world.step();
    }
    let mut wall = vec![];
    for _ in 0..steps {
        world.step();
        wall.push(world.last.wall_ns as f64 / 1e6);
    }
    let w = stat(wall);
    println!();
    println!(
        "window (JS oracle configuration: {} cars, {} ambient walkers, no camera): Rust {:.4} ms/step median, {:.4} p95, {:.3} max; JS TrafficSim alone {:.2} ms median, {:.2} p95 (recorded by the oracle run, same machine, other load varies)",
        world.regions[0].fleet.cars.len(),
        world.regions[0].crowd.walkers.len(),
        w.median,
        w.p95,
        w.max,
        oracle.run.step_ms.median,
        oracle.run.step_ms.p95
    );
    println!();
    println!("RESULT: {}", if pass { "PASS (1x main-thread cost under 1 ms)" } else { "FAIL" });
    if !pass {
        std::process::exit(1);
    }
    Ok(())
}
