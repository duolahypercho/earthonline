//! eo-server: the headless authoritative server, its bot clients and its benches.
//!
//!   eo-server [--bind 127.0.0.1:47800] [--snapshot-hz 20] [--budget-kbps 96] [--net-sim RTT,JITTER,LOSS] [--no-streets]
//!   eo-server bots --connect ADDR --count N [--around X,Z,R] [--drive 0.5] [--drive-mps 12] [--seconds S] [--threads 4]
//!   eo-server bench [--bots 1,16,64,256] [--seconds 20] [--warmup 8] [--out DIR]
//!   eo-server bandwidth [--seconds 20] [--out DIR]
//!   eo-server predict [--seconds 40] [--net-sim 80,20,2] [--out DIR]
//!
//! Every form takes `--ground-scale S` (the street size; default
//! the approved ground-plan deviation, 1.25). Clients and bots on another
//! ground plan are refused at the handshake. hills: and `--height-scale H`
//! (the ground heights; default: they follow the plan); a client on other
//! heights is refused too.
//!
//! The server forms (and `bots`) also take `--structures` / `--no-structures`
//! and `--tunnels` / `--no-tunnels` (the grade-separated roads; tunnels only
//! with structures; the last of a switch and its `--no-` form wins), as
//! eo-app does; a client or bot with other ones is refused at the handshake.
//!
//! Everything binds 127.0.0.1 unless told otherwise; nothing here opens a
//! firewall port or changes a system setting.

use anyhow::{Result, anyhow};
use eo_net::link::Conditions;
use eo_server::bots::{BotConfig, BotRun, RoadGraph, Spawn, bot_roads};
use eo_server::{Server, ServerConfig, load_with, sys};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({ "median": pct(v, 50.0), "p95": pct(v, 95.0), "p99": pct(v, 99.0), "max": pct(v, 100.0), "n": v.len() })
}

struct Flags {
    args: Vec<String>,
}

impl Flags {
    fn get(&self, name: &str) -> Option<String> {
        self.args.iter().position(|a| a == name).and_then(|i| self.args.get(i + 1).cloned())
    }
    fn has(&self, name: &str) -> bool {
        self.args.iter().any(|a| a == name)
    }
    fn num<T: std::str::FromStr>(&self, name: &str, d: T) -> T {
        self.get(name).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    /// `on` / `off` (a switch and its `--no-` form): the last one given wins.
    fn switch(&self, on: &str, off: &str, d: bool) -> bool {
        self.args.iter().rev().find(|a| *a == on || *a == off).map_or(d, |a| a == on)
    }
    /// `--structures` / `--no-structures` and `--tunnels` / `--no-tunnels`
    /// (tunnels only with structures), as eo-app reads them.
    fn structures(&self) -> Option<eo_structures::StructureOptions> {
        self.switch("--structures", "--no-structures", false).then(|| eo_structures::StructureOptions { tunnels: self.switch("--tunnels", "--no-tunnels", false) })
    }
}

fn out_dir(f: &Flags) -> std::path::PathBuf {
    f.get("--out").map(Into::into).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"))
}

fn server_config(f: &Flags) -> Result<ServerConfig> {
    let mut c = ServerConfig::default();
    if let Some(b) = f.get("--bind") {
        c.bind = b;
    }
    c.snapshot_hz = f.num("--snapshot-hz", c.snapshot_hz);
    c.budget_bytes_per_s = f.num("--budget-kbps", c.budget_bytes_per_s / 1000) * 1000;
    if let Some(s) = f.get("--net-sim") {
        c.conditions = Conditions::parse(&s).ok_or_else(|| anyhow!("--net-sim RTT,JITTER,LOSS"))?;
    }
    c.streets = !f.has("--no-streets");
    c.structures = f.structures();
    Ok(c)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let f = Flags { args: args.clone() };
    // `--ground-scale S` before anything reads the scale (no other thread yet).
    if let Some(v) = f.get("--ground-scale") {
        eo_data::groundscale::parse_scale(&v)?;
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("EO_GROUND_SCALE", &v) };
    }
    // `--height-scale H` (the ground heights; default: they follow the plan).
    if let Some(v) = f.get("--height-scale") {
        eo_data::groundscale::parse_scale(&v)?;
        // SAFETY: as above.
        unsafe { std::env::set_var("EO_HEIGHT_SCALE", &v) };
    }
    let s = eo_data::groundscale::resolve()?;
    let h = eo_data::groundscale::resolve_height()?;
    eprintln!("eo-server: ground plan x{s} ({}), heights x{h} ({})", eo_data::groundscale::source(), eo_data::groundscale::height_source());
    match args.first().map(|s| s.as_str()) {
        Some("bots") => bots_main(&f),
        Some("bench") => bench_main(&f),
        Some("bandwidth") => bandwidth_main(&f),
        Some("predict") => predict_main(&f),
        Some("-h") | Some("--help") => {
            println!("see the header of native/eo-server/src/main.rs");
            Ok(())
        }
        _ => serve_main(&f),
    }
}

fn serve_main(f: &Flags) -> Result<()> {
    let cfg = server_config(f)?;
    let t = Instant::now();
    let (game, source) = load_with(cfg.streets, cfg.structures)?;
    eprintln!("eo-server: world loaded in {:.1} s", t.elapsed().as_secs_f64());
    let mut server = Server::start(cfg, game, source)?;
    eprintln!("eo-server: listening on {} (UDP)", server.addr());
    let stop = AtomicBool::new(false);
    let mut last = Instant::now();
    let dt = Duration::from_secs_f64(1.0 / 60.0);
    let mut next = Instant::now();
    loop {
        let tt = server.tick();
        if last.elapsed().as_secs_f64() > 5.0 {
            last = Instant::now();
            let (cars, walkers) = server.regions.agents();
            eprintln!("tick {}: {} online, {} regions, {} cars, {} walkers, last tick {:.2} ms", server.tick, server.online(), server.regions.active(), cars, walkers, tt.total_ms);
        }
        next += dt;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
    Ok(())
}

fn graph() -> Result<Arc<RoadGraph>> {
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    Ok(Arc::new(RoadGraph::new(bot_roads(&city))))
}

fn bot_config(f: &Flags, server: std::net::SocketAddr) -> Result<BotConfig> {
    let spawn = match f.get("--around") {
        Some(s) => {
            let v: Vec<f64> = s.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
            Spawn::Around { x: v[0], z: v[1], radius: v.get(2).copied().unwrap_or(100.0) }
        }
        None => Spawn::Spread,
    };
    Ok(BotConfig {
        server,
        count: f.num("--count", 16usize),
        threads: f.num("--threads", 4usize),
        spawn,
        drive_share: f.num("--drive", 0.5),
        drive_mps: f.num("--drive-mps", 12.0),
        seed: f.num("--seed", 7u64),
        conditions: match f.get("--net-sim") {
            Some(s) => Conditions::parse(&s).ok_or_else(|| anyhow!("--net-sim"))?,
            None => Conditions::NONE,
        },
        send_hz: f.num("--send-hz", 30.0),
        structures: f.structures(),
    })
}

/// A bot process: connect, run, print one JSON line of client-side stats.
fn bots_main(f: &Flags) -> Result<()> {
    let server: std::net::SocketAddr = f.get("--connect").ok_or_else(|| anyhow!("bots --connect ADDR"))?.parse()?;
    let cfg = bot_config(f, server)?;
    let g = graph()?;
    let run = BotRun::start(cfg, g)?;
    let secs: f64 = f.num("--seconds", 1e9);
    let t = Instant::now();
    let stdin_closed = Arc::new(AtomicBool::new(false));
    {
        // The parent closes our stdin to stop us.
        let s = stdin_closed.clone();
        std::thread::spawn(move || {
            let mut b = String::new();
            let _ = std::io::stdin().read_line(&mut b);
            s.store(true, Ordering::Relaxed);
        });
    }
    while t.elapsed().as_secs_f64() < secs && !stdin_closed.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let st = run.stop();
    let kbps: Vec<f64> = st.bytes_per_s.iter().map(|b| *b as f64 / 1000.0).collect();
    println!("{}", json!({ "snapshots": st.snapshots, "decode_misses": st.decode_misses, "client_kBps": summary(&kbps) }));
    Ok(())
}

struct Measured {
    report: serde_json::Value,
}

/// Run a server with `bots` (in a child process) and measure it.
fn measure(cfg: ServerConfig, game: Arc<eo_net::game::GameWorld>, source: Arc<eo_sim::native::NativeSource>, bot_args: &[String], warmup_s: f64, seconds: f64, label: &str) -> Result<Measured> {
    // the bots claim the world's own grade-separated roads (else the handshake refuses them).
    let structures: Vec<&str> = match game.structures {
        Some(o) if o.tunnels => vec!["--structures", "--tunnels"],
        Some(_) => vec!["--structures"],
        None => vec![],
    };
    let mut server = Server::start(cfg.clone(), game, source)?;
    let addr = server.addr();
    let exe = std::env::current_exe()?;
    let mut child = std::process::Command::new(exe)
        .arg("bots")
        .arg("--connect")
        .arg(addr.to_string())
        .args(bot_args)
        .args(structures)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let dt = Duration::from_secs_f64(1.0 / 60.0);
    let mut next = Instant::now();
    let t0 = Instant::now();
    let mut measuring = false;
    let (mut cpu0, mut tick0, mut wall0) = (0.0, 0u32, Instant::now());
    let mut late = 0usize;
    let mut ticks: Vec<eo_server::TickTiming> = vec![];
    let mut mem_peak = sys::Memory::default();
    while t0.elapsed().as_secs_f64() < warmup_s + seconds {
        if !measuring && t0.elapsed().as_secs_f64() >= warmup_s {
            measuring = true;
            cpu0 = sys::process_cpu_s();
            tick0 = server.tick;
            wall0 = Instant::now();
            for s in server.sessions.iter_mut() {
                s.stats.bytes_per_s.clear();
            }
        }
        let t = server.tick();
        if measuring {
            ticks.push(t);
            if server.tick % 60 == 0 {
                let m = sys::memory();
                if m.working_set > mem_peak.working_set {
                    mem_peak = m;
                }
            }
        }
        next += dt;
        let now = Instant::now();
        if next > now {
            let w = next - now;
            if w > Duration::from_micros(1500) {
                std::thread::sleep(w - Duration::from_micros(1000));
            }
            while Instant::now() < next {
                std::hint::spin_loop();
            }
        } else {
            if measuring {
                late += 1;
            }
            if now - next > Duration::from_millis(200) {
                next = now;
            }
        }
    }
    let cpu = sys::process_cpu_s() - cpu0;
    let wall = wall0.elapsed().as_secs_f64();
    let nt = server.tick.wrapping_sub(tick0) as f64;
    drop(child.stdin.take());
    let out = child.wait_with_output()?;
    let client: serde_json::Value = String::from_utf8_lossy(&out.stdout).lines().last().and_then(|l| serde_json::from_str(l).ok()).unwrap_or(json!(null));
    let total: Vec<f64> = ticks.iter().map(|t| t.total_ms).collect();
    let players: Vec<f64> = ticks.iter().map(|t| t.players_ms).collect();
    let players_cpu: Vec<f64> = ticks.iter().map(|t| t.players_cpu_ms).collect();
    let regions: Vec<f64> = ticks.iter().map(|t| t.regions_ms).collect();
    let sim: Vec<f64> = ticks.iter().map(|t| t.sim_ms).collect();
    let sim_cpu: Vec<f64> = ticks.iter().map(|t| t.sim_cpu_ms).collect();
    let snap: Vec<f64> = ticks.iter().filter(|t| t.snapshot_tick).map(|t| t.snapshot_ms).collect();
    let net_in: Vec<f64> = ticks.iter().map(|t| t.net_in_ms).collect();
    let mut kbps: Vec<f64> = vec![];
    let mut per_client_median: Vec<f64> = vec![];
    for s in &server.sessions {
        let v: Vec<f64> = s.stats.bytes_per_s.iter().skip(1).map(|b| *b as f64 / 1000.0).collect();
        if !v.is_empty() {
            per_client_median.push(pct(&v, 50.0));
        }
        kbps.extend(v);
    }
    let (cars, walkers) = server.regions.agents();
    let online = server.online();
    let stood_in: u64 = server.sessions.iter().map(|s| s.stats.inputs_stood_in).sum();
    let corrections: u64 = server.sessions.iter().map(|s| s.stats.corrections_sent).sum();
    let deferred: u64 = server.sessions.iter().map(|s| s.stats.deferred).sum();
    let snapshots: u64 = server.sessions.iter().map(|s| s.stats.snapshots).sum();
    let entities: Vec<f64> = server.sessions.iter().map(|s| s.stats.entities as f64).collect();
    let rs = server.regions.stats.clone();
    let report = json!({
        "label": label,
        "players_online": online,
        "regions_active": server.regions.active(),
        "cars": cars,
        "walkers": walkers,
        "ticks": nt,
        "ticks_per_s": nt / wall,
        "ticks_late": late,
        "tick_ms": summary(&total),
        "tick_split_ms": { "net_in": summary(&net_in), "players_wall": summary(&players), "players_cpu_sum": summary(&players_cpu), "regions_update": summary(&regions), "sim_step_wall": summary(&sim), "sim_step_cpu_sum": summary(&sim_cpu), "snapshot_ticks_build_and_send": summary(&snap) },
        "process_cpu_ms_per_tick": cpu * 1e3 / nt.max(1.0),
        "process_cpu_cores": cpu / wall,
        "memory_mb": { "working_set": mem_peak.working_set as f64 / 1e6, "private": mem_peak.private as f64 / 1e6, "peak_working_set": mem_peak.peak_working_set as f64 / 1e6 },
        "server_to_client_kBps": summary(&kbps),
        "per_client_median_kBps": summary(&per_client_median),
        "entities_per_client": summary(&entities),
        "records_deferred_by_budget": deferred,
        "snapshots_sent": snapshots,
        "inputs_stood_in": stood_in,
        "corrections_sent": corrections,
        "regions": { "opened": rs.opened, "closed": rs.closed, "cars_handed_off": rs.cars_handed_off, "walkers_handed_off": rs.walkers_handed_off, "net_tiles_added": rs.net_tiles_added, "net_tiles_removed": rs.net_tiles_removed, "net_apply_ms_max": rs.net_apply_ms_max, "net_add_ms_max": rs.net_add_ms_max, "net_build_ms_max": rs.net_build_ms_max },
        "collision_tiles_cooked": server.tiles.resident(),
        "client_side": client,
    });
    Ok(Measured { report })
}

fn bench_main(f: &Flags) -> Result<()> {
    let counts: Vec<usize> = f.get("--bots").unwrap_or_else(|| "1,16,64,256".into()).split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
    let seconds: f64 = f.num("--seconds", 20.0);
    let warmup: f64 = f.num("--warmup", 8.0);
    let cfg = server_config(f)?;
    let t = Instant::now();
    let (game, source) = load_with(cfg.streets, cfg.structures)?;
    eprintln!("bench: world loaded in {:.1} s", t.elapsed().as_secs_f64());
    let mut reports = vec![];
    for n in counts {
        let mut c = cfg.clone();
        c.bind = "127.0.0.1:0".into();
        let bot_args: Vec<String> = vec!["--count".into(), n.to_string(), "--threads".into(), (n / 32).clamp(1, 6).to_string(), "--drive".into(), "0.5".into()];
        eprintln!("bench: {n} bots ...");
        let m = measure(c, game.clone(), source.clone(), &bot_args, warmup + (n as f64 / 32.0), seconds, &format!("{n}_bots_spread"))?;
        let r = &m.report;
        println!(
            "[{n} bots spread] online {} | tick ms median {:.3} p95 {:.3} max {:.3} | {:.1} ticks/s | CPU {:.3} ms/tick ({:.2} cores) | players wall {:.3} cpu {:.3} | sim wall {:.3} cpu {:.3} | snapshots {:.3} | mem {:.0} MB | per-client kB/s median {:.1} p95 {:.1} | regions {} cars {} walkers {}",
            r["players_online"], r["tick_ms"]["median"].as_f64().unwrap_or(f64::NAN), r["tick_ms"]["p95"].as_f64().unwrap_or(f64::NAN), r["tick_ms"]["max"].as_f64().unwrap_or(f64::NAN),
            r["ticks_per_s"].as_f64().unwrap_or(0.0), r["process_cpu_ms_per_tick"].as_f64().unwrap_or(0.0), r["process_cpu_cores"].as_f64().unwrap_or(0.0),
            r["tick_split_ms"]["players_wall"]["median"].as_f64().unwrap_or(0.0), r["tick_split_ms"]["players_cpu_sum"]["median"].as_f64().unwrap_or(0.0),
            r["tick_split_ms"]["sim_step_wall"]["median"].as_f64().unwrap_or(0.0), r["tick_split_ms"]["sim_step_cpu_sum"]["median"].as_f64().unwrap_or(0.0),
            r["tick_split_ms"]["snapshot_ticks_build_and_send"]["median"].as_f64().unwrap_or(0.0),
            r["memory_mb"]["working_set"].as_f64().unwrap_or(0.0),
            r["server_to_client_kBps"]["median"].as_f64().unwrap_or(0.0), r["server_to_client_kBps"]["p95"].as_f64().unwrap_or(0.0),
            r["regions_active"], r["cars"], r["walkers"],
        );
        reports.push(m.report);
    }
    let dir = out_dir(f);
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("net-server-bench.json");
    std::fs::write(&p, serde_json::to_vec_pretty(&json!({ "runs": reports, "machine_threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0) }))?)?;
    println!("wrote {}", p.display());
    Ok(())
}

/// Per-client bandwidth: a busy street (32 bots within 80 m of the bench
/// spawn on Folsom, walking and driving), and fast driving across tiles (16 bots at 25 m/s).
fn bandwidth_main(f: &Flags) -> Result<()> {
    let seconds: f64 = f.num("--seconds", 20.0);
    let cfg = server_config(f)?;
    let (game, source) = load_with(cfg.streets, cfg.structures)?;
    let mut reports = vec![];
    let at = eo_data::groundscale::plan([1539.0, -366.0]); // the spawn on the scaled plan
    let around = format!("{},{},80", at[0], at[1]);
    let busy = ["--count", "32", "--around", around.as_str(), "--drive", "0.4", "--drive-mps", "9", "--threads", "2"];
    let fast = ["--count", "16", "--drive", "1.0", "--drive-mps", "25", "--threads", "2"];
    for (label, a) in [("busy_street", &busy[..]), ("driving_fast", &fast[..])] {
        let mut c = cfg.clone();
        c.bind = "127.0.0.1:0".into();
        let args: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        let m = measure(c, game.clone(), source.clone(), &args, 10.0, seconds, label)?;
        let r = &m.report;
        println!(
            "[{label}] per-client server->client kB/s median {:.1} p95 {:.1} max {:.1}; entities per client median {} p95 {}; deferred {}; client-side kB/s median {}",
            r["server_to_client_kBps"]["median"].as_f64().unwrap_or(f64::NAN),
            r["server_to_client_kBps"]["p95"].as_f64().unwrap_or(f64::NAN),
            r["server_to_client_kBps"]["max"].as_f64().unwrap_or(f64::NAN),
            r["entities_per_client"]["median"],
            r["entities_per_client"]["p95"],
            r["records_deferred_by_budget"],
            r["client_side"]["client_kBps"]["median"],
        );
        reports.push(m.report);
    }
    let dir = out_dir(f);
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("net-bandwidth.json");
    std::fs::write(&p, serde_json::to_vec_pretty(&json!({ "runs": reports }))?)?;
    println!("wrote {}", p.display());
    Ok(())
}

/// Predicting clients (eo-net `Predictor`, the same as eo-app `--connect`)
/// against the server under simulated network conditions: position error
/// after reconciliation, corrections and their visible size.
fn predict_main(f: &Flags) -> Result<()> {
    let seconds: f64 = f.num("--seconds", 40.0);
    let cfg = server_config(f)?;
    let (game, source) = load_with(cfg.streets, cfg.structures)?;
    let mut reports = vec![];
    let profiles: Vec<(String, Conditions)> = match f.get("--net-sim") {
        Some(s) => vec![(format!("net_sim_{s}"), Conditions::parse(&s).ok_or_else(|| anyhow!("--net-sim"))?)],
        None => vec![("rtt80_jitter20_loss2".into(), Conditions::profile_80ms()), ("rtt160_jitter40_loss5_dup1".into(), Conditions::profile_harsh())],
    };
    for (label, cond) in profiles {
        for nudge in [false, true] {
            let r = eo_server::predict::run(&game, &source, cond, seconds, nudge)?;
            println!("[{label}{}] {}", if nudge { " + server-side shove" } else { "" }, r);
            reports.push(json!({ "label": label, "server_shove": nudge, "result": r }));
        }
    }
    let dir = out_dir(f);
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("net-predict.json");
    std::fs::write(&p, serde_json::to_vec_pretty(&json!({ "runs": reports }))?)?;
    println!("wrote {}", p.display());
    Ok(())
}
