//! End to end over UDP on 127.0.0.1: a real server thread and real clients
//! (`NetClient` + `Predictor`, the code eo-app `--connect` runs).
//!
//! * under loss, jitter (reordering) and duplicates every input still runs
//!   on the server, in order, and the prediction stays exact;
//! * a client whose link goes dark reconnects and resumes the same player;
//! * two clients near each other see each other's player where it is, and
//!   the same traffic: at any server tick both hold, every car and walker
//!   they both see has the same state.

use eo_net::client::{ClientConfig, Event, NetClient, Phase};
use eo_net::game::{GameWorld, rebuild_player};
use eo_net::link::{Conditions, Impair};
use eo_net::predict::Predictor;
use eo_net::proto::SpawnHint;
use eo_net::snapshot::{CLASS_CAR, CLASS_PLAYER, CLASS_WALKER, class_of, net_id};
use eo_net::wire::read_player_state;
use eo_player::StepInput;
use eo_server::ServerConfig;
use eo_server::predict::{Ctl, spawn_server};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn world() -> &'static (Arc<GameWorld>, Arc<eo_sim::native::NativeSource>) {
    static W: OnceLock<(Arc<GameWorld>, Arc<eo_sim::native::NativeSource>)> = OnceLock::new();
    W.get_or_init(|| eo_server::load(true).unwrap())
}

struct Harness {
    ctl: Arc<Mutex<Ctl>>,
    th: Option<std::thread::JoinHandle<()>>,
    addr: std::net::SocketAddr,
}

impl Harness {
    fn start() -> Self {
        let (g, s) = world();
        let ctl = Arc::new(Mutex::new(Ctl::default()));
        let th = spawn_server(ServerConfig { bind: "127.0.0.1:0".into(), player_threads: 2, sim_threads: 2, ..Default::default() }, g.clone(), s.clone(), ctl.clone());
        let addr = loop {
            if let Some(a) = ctl.lock().unwrap().addr {
                break a;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        Self { ctl, th: Some(th), addr }
    }
    fn stop(mut self) -> serde_json::Value {
        self.ctl.lock().unwrap().stop = true;
        self.th.take().unwrap().join().unwrap();
        self.ctl.lock().unwrap().stats.take().unwrap()
    }
}

/// A predicting client, driven frame by frame.
struct Player {
    net: NetClient,
    pred: Option<Predictor>,
    last_ack: u32,
    accepts: Vec<(u32, bool)>,
    out: Vec<(u32, eo_net::wire::NetInput)>,
}

impl Player {
    fn connect(addr: std::net::SocketAddr, name: &str, cond: Conditions, spawn: [f64; 3]) -> Self {
        let mut c = ClientConfig::new(addr, &format!("dev:{name}"));
        c.conditions = cond;
        c.spawn = Some(SpawnHint { x: spawn[0] as f32, z: spawn[1] as f32, yaw: spawn[2] as f32, driving: false });
        c.timeout_s = 2.0;
        Self { net: NetClient::connect(c).unwrap(), pred: None, last_ack: 0, accepts: vec![], out: vec![] }
    }

    fn frame(&mut self, input: &StepInput, yaw: f64, dt: f64) {
        for ev in self.net.take_events() {
            match ev {
                Event::Accepted(a) => {
                    let s = read_player_state(&a.state).unwrap();
                    let mut cw = world().0.collision_world(2);
                    let p = rebuild_player(&mut cw, &s, &a.car_spawns);
                    match self.pred.as_mut() {
                        Some(pr) => pr.reset(p, cw, a.input_seq),
                        None => self.pred = Some(Predictor::new(p, cw, a.input_seq)),
                    }
                    self.accepts.push((a.player_id, a.resumed));
                }
                Event::Correction(c) => {
                    if let Some(p) = self.pred.as_mut() {
                        p.correct(&c);
                    }
                }
                Event::Reliable(_) => {}
            }
        }
        let Some(p) = self.pred.as_mut() else { return };
        if let Some(o) = self.net.own() {
            if o.ack_input != self.last_ack {
                self.last_ack = o.ack_input;
                p.on_own(&o);
            }
        }
        p.player.yaw = yaw;
        self.out.clear();
        p.frame(input, dt, &mut self.out);
        self.net.push_inputs(&self.out, p.check(), p.applied);
    }

    fn connected(&self) -> bool {
        self.pred.is_some() && self.net.status().phase == Phase::Connected
    }
}

fn run_frames(players: &mut [&mut Player], seconds: f64, input: impl Fn(usize, f64) -> (StepInput, f64)) {
    let t0 = Instant::now();
    let mut last = Instant::now();
    while t0.elapsed().as_secs_f64() < seconds {
        let now = Instant::now();
        let dt = (now - last).as_secs_f64();
        last = now;
        for (k, p) in players.iter_mut().enumerate() {
            let (i, yaw) = input(k, t0.elapsed().as_secs_f64());
            p.frame(&i, yaw, dt);
        }
        std::thread::sleep(Duration::from_millis(6));
    }
}

fn wait_connected(players: &mut [&mut Player]) {
    let t0 = Instant::now();
    while !players.iter().all(|p| p.connected()) {
        assert!(t0.elapsed().as_secs_f64() < 60.0, "clients did not connect");
        for p in players.iter_mut() {
            p.frame(&StepInput::default(), 0.0, 0.0);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// the spawn on the shipped (scaled) plan.
static SPAWN: std::sync::LazyLock<[f64; 3]> = std::sync::LazyLock::new(|| {
    let p = eo_data::groundscale::plan([1539.0, -366.0]);
    [p[0], p[1], 1.5708]
});

#[test]
fn loss_reorder_and_duplicates_lose_no_input_and_keep_prediction_exact() {
    let h = Harness::start();
    let harsh = Conditions { send: Impair { latency_ms: 80.0, jitter_ms: 40.0, loss: 0.10, duplicate: 0.05 }, recv: Impair { latency_ms: 80.0, jitter_ms: 40.0, loss: 0.10, duplicate: 0.05 } };
    let mut p = Player::connect(h.addr, "lossy", harsh, *SPAWN);
    wait_connected(&mut [&mut p]);
    // Walk in a slow circle, sprinting and jumping now and then.
    run_frames(&mut [&mut p], 12.0, |_, t| (StepInput { move_y: 1.0, sprint: (t * 0.5).sin() > 0.3, jump: (t % 3.0) < 0.01, ..Default::default() }, t * 0.3));
    // Let the last inputs arrive.
    run_frames(&mut [&mut p], 1.5, |_, _| (StepInput::default(), 0.0));
    let st = p.net.status();
    let pred = p.pred.as_ref().unwrap();
    let server = h.stop();
    let run = server["inputs_run"].as_u64().unwrap();
    let stood = server["inputs_stood_in"].as_u64().unwrap();
    println!("10 % loss + 5 % duplicates + 40 ms jitter each way: client stepped {}, server ran {run} (stood in {stood}); snapshots {} lost {}; corrections {}; errors {:?}", pred.seq, st.snapshots, st.snapshots_lost, pred.stats.corrections, pred.stats.error_mm.iter().filter(|e| **e > 0.0).count());
    assert!(st.snapshots_lost > 0, "the link lost snapshots (the test means something)");
    assert_eq!(stood, 0, "no input had to be stood in for: every one arrived through the resends");
    assert!(run as i64 >= pred.seq as i64 - 30, "the server ran {run} of {} inputs", pred.seq);
    assert!(pred.stats.error_mm.len() > 100);
    assert!(pred.stats.error_mm.iter().all(|e| *e == 0.0), "the prediction matched the server at every acknowledged input");
    assert_eq!(pred.stats.corrections, 0);
}

/// A landmark lift over UDP: a client at Coit Tower's entrance taps interact
/// (the input's button), rides up, taps again and rides down; the server runs
/// the ride from the same inputs, so the prediction never needs a correction.
#[test]
fn a_landmark_lift_rides_over_the_network_with_no_correction() {
    let (g, _) = world();
    let l = g.buildings.decks.lifts.iter().find(|l| l.name.contains("Coit")).expect("the Coit Tower lift").clone();
    let h = Harness::start();
    let mut p = Player::connect(h.addr, "rider", Conditions::NONE, [l.bottom.x, l.bottom.z, l.bottom_yaw]);
    wait_connected(&mut [&mut p]);
    let tap = |t: f64, at: f64| t >= at && t < at + 0.02;
    run_frames(&mut [&mut p], 3.6, |_, t| (StepInput { interact: tap(t, 1.0), ..Default::default() }, l.bottom_yaw));
    let up = p.pred.as_ref().unwrap().player.position(&p.pred.as_ref().unwrap().cw);
    run_frames(&mut [&mut p], 3.6, |_, t| (StepInput { interact: tap(t, 0.5), ..Default::default() }, l.top_yaw));
    run_frames(&mut [&mut p], 1.5, |_, _| (StepInput::default(), 0.0));
    let pred = p.pred.as_ref().unwrap();
    let down = pred.player.position(&pred.cw);
    let _ = h.stop();
    println!("over UDP: up at {up:?} (top stop {:?}), down at {down:?}; corrections {}, acknowledged inputs {}", l.top, pred.stats.corrections, pred.stats.error_mm.len());
    assert!((up - l.top).length() < 1.0, "the ride up did not reach the gallery: {up:?}");
    assert!((down.x - l.bottom.x).hypot(down.z - l.bottom.z) < 1.0, "the ride down did not reach the entrance: {down:?}");
    assert!(pred.stats.error_mm.len() > 100);
    assert!(pred.stats.error_mm.iter().all(|e| *e == 0.0), "the prediction matched the server at every acknowledged input");
    assert_eq!(pred.stats.corrections, 0);
}

#[test]
fn a_client_that_loses_its_link_reconnects_as_the_same_player() {
    let h = Harness::start();
    let mut p = Player::connect(h.addr, "returner", Conditions::NONE, *SPAWN);
    wait_connected(&mut [&mut p]);
    run_frames(&mut [&mut p], 2.0, |_, _| (StepInput { move_y: 1.0, ..Default::default() }, 0.5));
    let id = p.net.player_id();
    let before = p.pred.as_ref().unwrap().player.character.feet();
    p.net.set_blackout(true);
    run_frames(&mut [&mut p], 3.5, |_, _| (StepInput::default(), 0.5));
    assert_eq!(p.net.status().phase, Phase::Reconnecting, "the client noticed the dead link");
    p.net.set_blackout(false);
    let t0 = Instant::now();
    while !(p.connected() && p.accepts.len() >= 2) {
        assert!(t0.elapsed().as_secs_f64() < 10.0, "no reconnect");
        p.frame(&StepInput::default(), 0.5, 0.016);
        std::thread::sleep(Duration::from_millis(5));
    }
    run_frames(&mut [&mut p], 1.0, |_, _| (StepInput { move_y: 1.0, ..Default::default() }, 0.5));
    let after = p.pred.as_ref().unwrap().player.character.feet();
    let st = p.net.status();
    let server = h.stop();
    println!("reconnect: accepts {:?}, moved {:.2} m across it, reconnects {}, server {server}", p.accepts, (after - before).length(), st.reconnects);
    assert_eq!(p.accepts[1], (id, true), "resumed the same player");
    assert_eq!(st.reconnects, 1);
    assert!((after - before).length() < 15.0, "the body was kept where it was");
}

#[test]
fn two_clients_see_each_other_and_the_same_traffic() {
    let h = Harness::start();
    let a_spawn = *SPAWN;
    let b_spawn = [SPAWN[0] + 6.0, SPAWN[1] + 2.0, -1.5708];
    let mut a = Player::connect(h.addr, "ada", Conditions::profile_80ms(), a_spawn);
    let mut b = Player::connect(h.addr, "bo", Conditions::profile_80ms(), b_spawn);
    wait_connected(&mut [&mut a, &mut b]);
    run_frames(&mut [&mut a, &mut b], 8.0, |k, t| (StepInput { move_x: if k == 0 { (t * 0.8).sin() } else { 0.0 }, ..Default::default() }, 0.0));
    // Each sees the other's player near where that one predicts itself.
    let (ida, idb) = (a.net.player_id(), b.net.player_id());
    for (me, other, oid) in [(&a, &b, idb), (&b, &a, ida)] {
        let v = me.net.view(Instant::now()).expect("a view");
        let e = v.b.entities.iter().find(|e| e.0 == net_id(CLASS_PLAYER, oid)).expect("the other player is replicated");
        let pos = e.1.pos();
        let truth = other.pred.as_ref().unwrap().player.character.feet();
        let d = (pos[0] - truth.x).hypot(pos[2] - truth.z);
        assert!(d < 1.0, "the other player is drawn {d:.2} m from where it is");
    }
    // The same traffic: snapshots for the same server tick agree on every shared car and walker.
    let mut by_tick: HashMap<u32, Arc<eo_net::snapshot::Decoded>> = HashMap::new();
    let mut compared = (0usize, 0usize);
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f64() < 4.0 {
        for (k, p) in [&mut a, &mut b].into_iter().enumerate() {
            p.frame(&StepInput::default(), 0.0, 0.008);
            if let Some(v) = p.net.view(Instant::now()) {
                let d = v.b.clone();
                let key = d.header.tick;
                if k == 0 {
                    by_tick.insert(key, d);
                } else if let Some(other) = by_tick.get(&key) {
                    let theirs: HashMap<u32, eo_net::snapshot::QState> = other.entities.iter().copied().collect();
                    for (id, s) in d.entities.iter() {
                        if matches!(class_of(*id), CLASS_CAR | CLASS_WALKER) {
                            if let Some(t) = theirs.get(id) {
                                assert_eq!(s, t, "entity {id:#x} differs between the two clients at tick {key}");
                                if class_of(*id) == CLASS_CAR {
                                    compared.0 += 1;
                                } else {
                                    compared.1 += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = h.stop();
    println!("compared {} car states and {} walker states at equal ticks: all equal", compared.0, compared.1);
    assert!(compared.0 > 100 && compared.1 > 100, "too few shared entities compared: {compared:?}");
}

/// a client on another ground plan (street size) is refused at
/// its request, before the server keeps anything for it; one on the server's
/// own is challenged as usual.
#[test]
fn a_client_on_another_ground_plan_is_refused() {
    use eo_net::proto::{self, DenyReason, Hello, Kind};
    let h = Harness::start();
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let ask = |ground: u32, salt: u64| -> (Kind, Vec<u8>) {
        let mut out = vec![];
        Hello { client_salt: salt, version: proto::PROTOCOL_VERSION, ground, token: b"dev:gs".to_vec(), ..Default::default() }.write(Kind::Request, &mut out);
        sock.send_to(&out, h.addr).unwrap();
        let mut buf = [0u8; 1500];
        let (n, _) = sock.recv_from(&mut buf).expect("the server answers a request");
        let (k, mut r) = proto::peek(&buf[..n]).expect("a protocol datagram");
        let mut rest = vec![];
        while let Ok(b) = r.u8() {
            rest.push(b);
        }
        (k, rest)
    };
    let own = proto::ground_scale_ppm();
    let other = if own == 1_000_000 { 1_250_000 } else { 1_000_000 };
    let (k, rest) = ask(other, 77);
    assert_eq!(k, Kind::Deny, "another ground plan is refused");
    assert_eq!(rest[8], DenyReason::World as u8);
    let (k, _) = ask(own, 78);
    assert_eq!(k, Kind::Challenge, "the server's own ground plan is challenged");
    h.stop();
}
