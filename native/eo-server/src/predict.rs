//! A predicting client against a live server under simulated network
//! conditions: the same client code eo-app `--connect` runs (`NetClient`
//! + `Predictor`), scripted input (walk to the car, get in, drive with
//! turns and stops, get out, walk), on a real-time loop at about 120 Hz.
//! Optionally the server shoves the player 1.5 m sideways halfway (an
//! event the client cannot predict) to measure a real correction.

use crate::{Server, ServerConfig};
use eo_net::client::{ClientConfig, Event, NetClient, Phase};
use eo_net::game::{GameWorld, rebuild_player};
use eo_net::link::Conditions;
use eo_net::predict::Predictor;
use eo_net::proto::SpawnHint;
use eo_net::wire::read_player_state;
use eo_player::{Mode, StepInput};
use glam::DVec3;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Ctl {
    pub addr: Option<SocketAddr>,
    pub shove: Option<DVec3>,
    pub stop: bool,
    pub stats: Option<serde_json::Value>,
}

/// A server on its own thread (it is built there), ticking at 60 Hz until `ctl.stop`.
pub fn spawn_server(cfg: ServerConfig, game: Arc<GameWorld>, source: Arc<eo_sim::native::NativeSource>, ctl: Arc<Mutex<Ctl>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("eo-server-main".into())
        .spawn(move || {
            let mut server = Server::start(cfg, game, source).expect("server start");
            ctl.lock().unwrap().addr = Some(server.addr());
            let dt = Duration::from_secs_f64(1.0 / 60.0);
            let mut next = Instant::now();
            loop {
                {
                    let mut c = ctl.lock().unwrap();
                    if c.stop {
                        let stood: u64 = server.sessions.iter().map(|s| s.stats.inputs_stood_in).sum();
                        let corr: u64 = server.sessions.iter().map(|s| s.stats.corrections_sent).sum();
                        let mism: u64 = server.sessions.iter().map(|s| s.stats.mismatches).sum();
                        let run: u64 = server.sessions.iter().map(|s| s.stats.inputs_run).sum();
                        c.stats = Some(json!({ "inputs_run": run, "inputs_stood_in": stood, "mismatches": mism, "corrections_sent": corr }));
                        return;
                    }
                    if let Some(d) = c.shove.take() {
                        for s in server.sessions.iter_mut() {
                            s.nudge = Some(d);
                        }
                    }
                }
                server.tick();
                next += dt;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
        })
        .expect("spawn server thread")
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

/// The scripted player of the prediction bench and the determinism test:
/// walk to the car parked 4 m to the left, drive the eo-player route
/// (`routes::walk_route`, pure pursuit at 40 km/h) for 60 % of the run,
/// brake, get out, walk on along the route, sprinting in spells, jumping now and then.
pub struct Script {
    pub route: Vec<[f64; 2]>,
    drive: eo_player::routes::DrivePilot,
    walk: Option<eo_player::routes::WalkPilot>,
    pub car: DVec3,
}

impl Script {
    pub fn new(route: Vec<[f64; 2]>, car: [f64; 2]) -> Self {
        Self { drive: eo_player::routes::DrivePilot::new(route.clone(), 40.0 / 3.6), route, walk: None, car: DVec3::new(car[0], 0.0, car[1]) }
    }

    /// The input for step `k` of `total`, quantised as sent.
    pub fn input(&mut self, k: usize, total: usize, p: &mut eo_player::Player, cw: &eo_player::world::CollisionWorld) -> eo_net::wire::NetInput {
        let s = self.step_input(k, total, p, cw);
        eo_net::wire::NetInput::quantize(&s, p.yaw)
    }

    /// The device input for step `k` of `total` (sets the camera yaw as a player's mouse would).
    pub fn step_input(&mut self, k: usize, total: usize, p: &mut eo_player::Player, cw: &eo_player::world::CollisionWorld) -> StepInput {
        let u = k as f64 / total.max(1) as f64;
        let mut s = StepInput::default();
        match p.mode {
            Mode::OnFoot if self.walk.is_none() && u < 0.6 => {
                let d = self.car - p.character.feet();
                p.yaw = d.x.atan2(d.z);
                if d.x.hypot(d.z) > 2.0 {
                    s.move_y = 1.0;
                } else {
                    s.vehicle_toggle = k % 2 == 0;
                }
            }
            Mode::OnFoot => {
                if self.walk.is_none() {
                    let here = p.position(cw);
                    let i = self.route.iter().enumerate().min_by(|a, b| ((a.1[0] - here.x).hypot(a.1[1] - here.z)).total_cmp(&(b.1[0] - here.x).hypot(b.1[1] - here.z))).map(|e| e.0).unwrap_or(0);
                    self.walk = Some(eo_player::routes::WalkPilot::new(self.route[i..].to_vec(), false));
                }
                let w = self.walk.as_mut().unwrap();
                w.sprint = (k / 180) % 2 == 1;
                s = w.input(p, cw);
                s.jump = k % 150 == 0;
            }
            Mode::Driving(i) => {
                if u < 0.6 {
                    s = self.drive.input(p, cw);
                } else {
                    let v = p.vehicles[i].forward_speed(cw);
                    s.brake = 1.0;
                    s.vehicle_toggle = v.abs() < 0.3 && k % 2 == 0;
                }
            }
        }
        s
    }
}

/// The route the script follows, and where to stand at its start (a street, so the car beside fits).
pub fn script_route(game: &GameWorld) -> (Vec<[f64; 2]>, [f64; 3]) {
    let r = eo_player::routes::walk_route(&game.city, &game.elevation);
    let p = r.path.clone();
    let yaw = (p[1][0] - p[0][0]).atan2(p[1][1] - p[0][1]);
    (p.clone(), [p[0][0], p[0][1], yaw])
}


/// One run: returns the measurements.
pub fn run(game: &Arc<GameWorld>, source: &Arc<eo_sim::native::NativeSource>, cond: Conditions, seconds: f64, shove: bool) -> anyhow::Result<serde_json::Value> {
    let ctl = Arc::new(Mutex::new(Ctl::default()));
    let cfg = ServerConfig { bind: "127.0.0.1:0".into(), player_threads: 2, sim_threads: 2, structures: game.structures, ..Default::default() };
    let th = spawn_server(cfg.clone(), game.clone(), source.clone(), ctl.clone());
    let addr = loop {
        if let Some(a) = ctl.lock().unwrap().addr {
            break a;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let (route, spawn) = script_route(game);
    let mut cc = ClientConfig::new(addr, "dev:predict");
    cc.conditions = cond;
    cc.structures = game.structures; // the world's own grade-separated roads
    cc.spawn = Some(SpawnHint { x: spawn[0] as f32, z: spawn[1] as f32, yaw: spawn[2] as f32, driving: false });
    let mut client = NetClient::connect(cc)?;
    let mut pred: Option<Predictor> = None;
    let mut script: Option<Script> = None;
    let t_connect = Instant::now();
    let mut connect_ms = 0.0;
    let mut frame_net_ms: Vec<f64> = vec![];
    let mut frame_step_ms: Vec<f64> = vec![];
    let mut drawn_offset_m: Vec<f64> = vec![];
    let mut out = vec![];
    let mut last = Instant::now();
    let mut t_play = 0.0;
    let mut shoved = false;
    let mut last_ack = 0;
    let mut dice = eo_net::link::Dice::new(11);
    let mut shown = vec![];
    let total_steps = (seconds * 60.0) as usize;
    let (mut walked, mut drove) = (0usize, 0usize);
    while t_play < seconds {
        let now = Instant::now();
        let dt = (now - last).as_secs_f64();
        last = now;
        let t0 = Instant::now();
        let mut joined_this_frame = false;
        for ev in client.take_events() {
            match ev {
                Event::Accepted(a) => {
                    // Joining: build the local physics world (a loading step, not a frame).
                    let state = read_player_state(&a.state)?;
                    let mut cw = game.collision_world(2);
                    let player = rebuild_player(&mut cw, &state, &a.car_spawns);
                    script = Some(Script::new(route.clone(), [a.car_spawns[0][0] as f64, a.car_spawns[0][1] as f64]));
                    pred = Some(Predictor::new(player, cw, a.input_seq));
                    connect_ms = t_connect.elapsed().as_secs_f64() * 1e3;
                    joined_this_frame = true;
                }
                Event::Correction(c) => {
                    if let Some(p) = pred.as_mut() {
                        p.correct(&c);
                    }
                }
                Event::Reliable(_) => {}
            }
        }
        let own = client.own();
        let net_a = t0.elapsed().as_secs_f64() * 1e3;
        let (Some(p), Some(sc)) = (pred.as_mut(), script.as_mut()) else {
            if t_connect.elapsed().as_secs_f64() > 60.0 {
                anyhow::bail!("no accept in 60 s");
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        if joined_this_frame {
            last = Instant::now();
            continue;
        }
        if let Some(o) = own {
            if o.ack_input != last_ack {
                last_ack = o.ack_input;
                p.on_own(&o);
            }
        }
        t_play += dt;
        let input = sc.step_input(p.stats.steps as usize, total_steps, &mut p.player, &p.cw);
        out.clear();
        let ts = Instant::now();
        p.frame(&input, dt, &mut out);
        let step_ms = ts.elapsed().as_secs_f64() * 1e3;
        match p.player.mode {
            Mode::OnFoot => walked += out.len(),
            Mode::Driving(_) => drove += out.len(),
        }
        drawn_offset_m.push(p.offset(dt).length());
        let t1 = Instant::now();
        client.push_inputs(&out, p.check(), p.applied);
        if let Some(v) = client.view(Instant::now()) {
            eo_net::snapshot::interpolate(&v.a, &v.b, v.tick, 15.0, &mut shown);
        }
        let net_b = t1.elapsed().as_secs_f64() * 1e3;
        frame_net_ms.push(net_a + net_b);
        frame_step_ms.push(step_ms);
        if shove && !shoved && t_play > seconds / 2.0 {
            shoved = true;
            ctl.lock().unwrap().shove = Some(DVec3::new(1.5, 0.0, 0.0));
        }
        // About 120 frames a second, with some jitter, as a render loop.
        std::thread::sleep(Duration::from_micros(7000 + (dice.next_f64() * 2000.0) as u64));
    }
    let st = client.status();
    let p = pred.take().unwrap();
    client.disconnect();
    ctl.lock().unwrap().stop = true;
    th.join().ok();
    let server = ctl.lock().unwrap().stats.take();
    let e = &p.stats.error_mm;
    let nonzero = e.iter().filter(|x| **x > 0.0).count();
    let shown_nonzero: Vec<f64> = drawn_offset_m.iter().copied().filter(|x| *x > 0.0).collect();
    Ok(json!({
        "seconds": seconds,
        "phase": format!("{:?}", st.phase),
        "connect_ms": connect_ms,
        "rtt_ms": st.rtt_ms,
        "interp_delay_ms": st.interp_delay_ms,
        "snapshots": st.snapshots,
        "snapshots_lost": st.snapshots_lost,
        "predicted_steps": p.stats.steps,
        "steps_walking": walked,
        "steps_driving": drove,
        "error_after_reconciliation_mm": { "median": pct(e, 50.0), "p95": pct(e, 95.0), "p99": pct(e, 99.0), "max": pct(e, 100.0), "samples": e.len(), "nonzero": nonzero },
        "corrections": p.stats.corrections,
        "correction_jump_m": p.stats.correction_m,
        "drawn_correction_offset_m": { "frames_with_offset": shown_nonzero.len(), "max": pct(&shown_nonzero, 100.0), "p95_of_those": pct(&shown_nonzero, 95.0) },
        "replayed_steps": p.stats.replayed_steps,
        "replay_ms_max": p.stats.replay_ms_max,
        "main_thread_net_ms_per_frame": { "median": pct(&frame_net_ms, 50.0), "p95": pct(&frame_net_ms, 95.0), "max": pct(&frame_net_ms, 100.0) },
        "main_thread_predicted_step_ms_per_frame": { "median": pct(&frame_step_ms, 50.0), "p95": pct(&frame_step_ms, 95.0), "max": pct(&frame_step_ms, 100.0) },
        "entities_shown_last_frame": shown.len(),
        "server": server,
    }))
}

pub fn is_connected(c: &NetClient) -> bool {
    c.status().phase == Phase::Connected
}
