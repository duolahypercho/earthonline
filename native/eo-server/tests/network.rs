//! End to end over UDP on 127.0.0.1: a real server thread and real clients
//! (`NetClient` + `Predictor`, the code eo-app `--connect` runs).
//!
//! * under loss, jitter (reordering) and duplicates every input still runs
//!   on the server, in order, and the prediction stays exact;
//! * a client whose link goes dark reconnects and resumes the same player;
//! * two clients near each other see each other's player where it is, and
//!   the same traffic: at any server tick both hold, every car and walker
//!   they both see has the same state;
//! * presence at a landmark: each knows the other's name, sees its emote,
//!   both draw the same sky, and a fast trip lands the traveller at the
//!   viewpoint for both (the server validates it and refuses a bad one).

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
    fn start() -> Self { Self::with_config(ServerConfig::default()) }

    fn with_config(config: ServerConfig) -> Self {
        let (g, s) = world();
        Self::with_world(config, g.clone(), s.clone())
    }

    fn with_world(config: ServerConfig, game: Arc<GameWorld>, source: Arc<eo_sim::native::NativeSource>) -> Self {
        let ctl = Arc::new(Mutex::new(Ctl::default()));
        let th = spawn_server(ServerConfig { bind: "127.0.0.1:0".into(), player_threads: 2, sim_threads: 2, ..config }, game, source, ctl.clone());
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
    reliable: Vec<Vec<u8>>,
}

impl Player {
    fn connect(addr: std::net::SocketAddr, name: &str, cond: Conditions, spawn: [f64; 3]) -> Self {
        Self::join(addr, name, cond, spawn, None)
    }

    fn join(addr: std::net::SocketAddr, name: &str, cond: Conditions, spawn: [f64; 3], vehicle: Option<eo_player::vehicle::VehicleConfig>) -> Self {
        let mut c = ClientConfig::new(addr, &format!("dev:{name}"));
        c.vehicle = vehicle.unwrap_or_default();
        c.conditions = cond;
        c.spawn = Some(SpawnHint { x: spawn[0] as f32, z: spawn[1] as f32, yaw: spawn[2] as f32, driving: vehicle.is_some() });
        c.timeout_s = 2.0;
        Self { net: NetClient::connect(c).unwrap(), pred: None, last_ack: 0, accepts: vec![], out: vec![], reliable: vec![] }
    }

    fn frame(&mut self, input: &StepInput, yaw: f64, dt: f64) {
        for ev in self.net.take_events() {
            match ev {
                Event::Accepted(a) => {
                    let s = read_player_state(&a.state).unwrap();
                    let mut cw = world().0.collision_world(2);
                    let p = rebuild_player(&mut cw, &s, &a.car_spawns);
                    match self.pred.as_mut() {
                        Some(pr) => pr.reset_with_weather(p, cw, a.input_seq, a.weather),
                        None => self.pred = Some(Predictor::with_weather(p, cw, a.input_seq, a.weather)),
                    }
                    self.accepts.push((a.player_id, a.resumed));
                }
                Event::Correction(c) => {
                    if let Some(p) = self.pred.as_mut() {
                        p.correct(&c);
                    }
                }
                Event::Reliable(m) => self.reliable.push(m),
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
    h.ctl.lock().unwrap().sky = Some((8.0, 1.0, eo_net::presence::weather_code("rain").unwrap(), 19));
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
    assert_eq!(p.pred.as_ref().unwrap().player.weather.rain_mm_h, 6.0);
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
    verify_shared_traffic(Harness::start());
}

#[test]
fn two_clients_replicate_traffic_from_the_drawn_street_plan() {
    let (game, source) = eo_server::load_with_traffic_plan(true, None, true, true).unwrap();
    let config = ServerConfig { traffic_contact: true, ..Default::default() };
    verify_shared_traffic(Harness::with_world(config, game, source));
}

fn verify_shared_traffic(h: Harness) {
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

/// a client whose grade-separated roads (`--structures`, `--tunnels`) are not
/// the server's world's is refused at its request, the same way as another
/// ground plan, and told the server's own; one with the server's own (here
/// none) is challenged as usual. The real client (eo-app `--connect`) with
/// `--structures` against this server without them ends denied.
#[test]
fn a_client_with_other_structures_is_refused() {
    use eo_net::proto::{self, DenyReason, Hello, Kind};
    use eo_structures::StructureOptions;
    let h = Harness::start();
    let own = proto::structures_code(world().0.structures);
    assert_eq!(own, 0, "the harness world has no structures");
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let ask = |structures: u8, salt: u64| -> (Kind, Vec<u8>) {
        let mut out = vec![];
        Hello { client_salt: salt, version: proto::PROTOCOL_VERSION, ground: proto::ground_scale_ppm(), structures, token: b"dev:st".to_vec(), ..Default::default() }.write(Kind::Request, &mut out);
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
    for (salt, other) in [(91, Some(StructureOptions { tunnels: false })), (92, Some(StructureOptions { tunnels: true }))] {
        let (k, rest) = ask(proto::structures_code(other), salt);
        assert_eq!(k, Kind::Deny, "{other:?} is refused");
        assert_eq!(rest[8], DenyReason::Structures as u8);
        assert_eq!(rest[9], own, "the denial names the server's own setting");
    }
    let (k, _) = ask(own, 93);
    assert_eq!(k, Kind::Challenge, "the server's own structures setting is challenged");
    // the real client.
    let mut c = ClientConfig::new(h.addr, "dev:structures");
    c.structures = Some(StructureOptions { tunnels: false });
    let net = NetClient::connect(c).unwrap();
    let t0 = Instant::now();
    while net.status().phase != Phase::Denied(DenyReason::Structures as u8) {
        assert!(t0.elapsed().as_secs_f64() < 10.0, "the client was not refused: {:?}", net.status().phase);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(net);
    h.stop();
}

/// Two friends at a landmark: names, an emote, the same sky, and a fast trip.
#[test]
fn friends_share_names_emotes_the_sky_and_a_fast_trip() {
    use eo_net::presence::{Emote, unpack_anim, viewpoints};
    use eo_net::proto::{TravelReply, TravelResult, msg, read_roster};
    let h = Harness::start();
    let a_spawn = *SPAWN;
    let b_spawn = [SPAWN[0] + 5.0, SPAWN[1] + 1.0, -1.5708];
    let mut a = Player::connect(h.addr, "ada", Conditions::profile_80ms(), a_spawn);
    let mut b = Player::connect(h.addr, "bo", Conditions::profile_80ms(), b_spawn);
    wait_connected(&mut [&mut a, &mut b]);
    run_frames(&mut [&mut a, &mut b], 1.5, |_, _| (StepInput::default(), 0.0));
    let (ida, idb) = (a.net.player_id(), b.net.player_id());
    // Names: each has heard of the other (whoever joined first).
    let names = |p: &Player| -> HashMap<u32, String> { p.reliable.iter().filter_map(|m| read_roster(m)).filter(|r| r.0).map(|r| (r.1, r.2)).collect() };
    assert_eq!(names(&a).get(&idb).map(String::as_str), Some("bo"), "ada knows bo's name");
    assert_eq!(names(&b).get(&ida).map(String::as_str), Some("ada"), "bo knows ada's name");
    // Bo waves; Ada sees it on bo's player.
    b.net.command(vec![msg::EMOTE, Emote::Wave as u8]);
    run_frames(&mut [&mut a, &mut b], 1.0, |_, _| (StepInput::default(), 0.0));
    let v = a.net.view(Instant::now()).unwrap();
    let e = v.b.entities.iter().find(|e| e.0 == net_id(CLASS_PLAYER, idb)).unwrap();
    assert_eq!(unpack_anim(e.1.anim).0, Emote::Wave, "ada sees bo wave");
    // The same sky: the two headers of one server tick carry the same clock.
    let (va, vb) = (a.net.view(Instant::now()).unwrap(), b.net.view(Instant::now()).unwrap());
    let (sa, sb) = (va.b.header.sky.hour_after(0.0), vb.b.header.sky.hour_after((va.b.header.tick as f64 - vb.b.header.tick as f64) / 60.0));
    assert!(eo_net::presence::hour_delta(sa, sb).abs() < 1e-6 && va.b.header.sky.weather == vb.b.header.sky.weather, "one sky: {sa} h against {sb} h");
    // A trip to nowhere is refused; one to Coit Tower lands bo at the viewpoint for both.
    let coit = viewpoints().unwrap().iter().position(|v| v.id == "coit-tower").unwrap() as u16;
    b.net.command(vec![msg::TRAVEL, 0xFF, 0x7F]);
    b.net.command(vec![msg::TRAVEL, coit as u8, (coit >> 8) as u8]);
    run_frames(&mut [&mut a, &mut b], 3.0, |_, _| (StepInput::default(), 0.0));
    let replies: Vec<TravelReply> = b.reliable.iter().filter_map(|m| TravelReply::read(m)).collect();
    assert!(replies.iter().any(|r| r.result == TravelResult::Unknown), "an unknown viewpoint is refused: {replies:?}");
    let ok = replies.iter().find(|r| r.result == TravelResult::Accepted).expect("the trip to Coit Tower is accepted");
    let feet = b.pred.as_ref().unwrap().player.character.feet();
    let d = (feet.x - ok.arrival[0] as f64).hypot(feet.z - ok.arrival[1] as f64);
    assert!(d < 0.5, "bo stands {d:.2} m from the arrival point");
    // Ada follows; there she sees bo where he stands, beside her on the ring, not in her.
    a.net.command(vec![msg::TRAVEL, coit as u8, (coit >> 8) as u8]);
    run_frames(&mut [&mut a, &mut b], 3.0, |_, _| (StepInput::default(), 0.0));
    let v = a.net.view(Instant::now()).unwrap();
    let e = v.b.entities.iter().find(|e| e.0 == net_id(CLASS_PLAYER, idb)).expect("ada, at Coit Tower too, holds bo");
    let seen = e.1.pos();
    assert!((seen[0] - feet.x).hypot(seen[2] - feet.z) < 1.0, "ada sees bo at Coit Tower");
    let fa = a.pred.as_ref().unwrap().player.character.feet();
    let apart = (fa.x - feet.x).hypot(fa.z - feet.z);
    assert!(apart > 0.8 && apart < 12.0, "the two arrive {apart:.2} m apart");
    // The emote ended with the trip, and no input was lost on the way.
    assert_eq!(unpack_anim(e.1.anim).0, Emote::None);
    let st = h.stop();
    println!("presence: names, a wave, one sky, a trip of {:.0} m; server {st}", (feet.x - b_spawn[0]).hypot(feet.z - b_spawn[1]));
}

#[test]
fn a_vehicle_impact_moves_both_authoritative_clients() {
    run_vehicle_impact("clear", Conditions::NONE, 0);
}

#[test]
fn a_loaded_van_and_a_hatchback_reconcile_their_contact_on_a_storm_wet_road() {
    run_vehicle_impact("storm", Conditions::parse("90,8,0").unwrap(), 600);
}

fn run_vehicle_impact(weather: &str, link: Conditions, van_cargo: u16) {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let mode = eo_net::presence::weather_code(weather).unwrap();
    let h = Harness::with_config(ServerConfig { weather: mode, ..Default::default() });
    let game = &world().0;
    let route = eo_player::routes::drive_route(&game.city, &game.elevation);
    let [x, z] = route.path[0];
    let yaw = (route.path[1][0] - x).atan2(route.path[1][1] - z);
    let vehicle = |model, cargo_kg| Some(VehicleConfig { model, occupants_kg: 75, cargo_kg });
    let mut a = Player::join(h.addr, "driver", link, [x,z,yaw], vehicle(VehicleModel::DeliveryVan, van_cargo));
    let mut b = Player::join(h.addr, "waiting", link, [x+9.0*yaw.sin(), z+9.0*yaw.cos(), yaw], vehicle(VehicleModel::Hatchback, 0));
    wait_connected(&mut [&mut a,&mut b]);
    let position = |p: &Player| {
        let state = p.pred.as_ref().unwrap();
        state.player.vehicles[0].datum(&state.cw)
    };
    let initial = position(&b);
    run_frames(&mut [&mut a,&mut b], 5.5, |which,t| {
        (StepInput { throttle: if which==0 && t < 3.0 { 0.8 } else { 0.0 }, ..Default::default() }, yaw)
    });
    let moved = position(&b) - initial;
    let corrections = [a.pred.as_ref().unwrap().stats.corrections,b.pred.as_ref().unwrap().stats.corrections];
    let expected = eo_net::weather::RoadWeather::new(eo_net::weather::WeatherEpoch::new(mode, 0, 0, 0)).at(0);
    for (driver, cargo) in [(&a, van_cargo), (&b, 0)] {
        let peer = driver.pred.as_ref().unwrap();
        assert_eq!(peer.player.weather, expected);
        assert_eq!(peer.player.vehicles[0].spec.config.cargo_kg, cargo);
        assert!(peer.stats.error_mm.len() > 30, "expected acknowledgements during and after contact");
        if van_cargo > 0 {
            assert!(peer.stats.replayed_steps > 0, "a delayed contact snapshot must replay pending inputs");
        }
    }
    let stats = h.stop();
    let contacts = stats["vehicle_contact_ticks"].as_array().unwrap();
    eprintln!("{weather}, van cargo {van_cargo} kg, contacts {contacts:?}, applied {corrections:?}, hatchback displacement {moved:?}");
    assert_eq!(contacts.len(), 2);
    assert!(contacts.iter().all(|x| x.as_u64().unwrap() > 0) && corrections.iter().all(|n| *n > 0));
    assert!(moved.x*yaw.sin() + moved.z*yaw.cos() > 0.4);
}

#[test]
fn driving_uses_the_servers_rain_at_join_and_its_storm_during_replay() {
    let rain = eo_net::presence::weather_code("rain").unwrap();
    let storm = eo_net::presence::weather_code("storm").unwrap();
    let h = Harness::with_config(ServerConfig { weather: rain, ..Default::default() });
    let game = &world().0;
    let route = eo_player::routes::drive_route(&game.city, &game.elevation);
    let [x, z] = route.path[0];
    let yaw = (route.path[1][0] - x).atan2(route.path[1][1] - z);
    let link = Conditions::parse("90,8,0").unwrap();
    let mut p = Player::join(h.addr, "rain-driver", link, [x, z, yaw], Some(eo_player::vehicle::VehicleConfig { model: eo_player::vehicle::VehicleModel::Sedan, ..Default::default() }));
    wait_connected(&mut [&mut p]);
    let rain_state = p.pred.as_ref().unwrap().player.weather;
    assert_eq!((rain_state.rain_mm_h, rain_state.wind), (6.0, [4.4, 1.1]));
    let drive = |_: usize, _: f64| (StepInput { throttle: 0.7, ..Default::default() }, yaw);
    run_frames(&mut [&mut p], 2.5, drive);
    let before = p.pred.as_ref().unwrap();
    assert!(before.player.vehicles[0].forward_speed(&before.cw) > 2.0, "car speed {}, mode {:?}", before.player.vehicles[0].forward_speed(&before.cw), before.player.mode);
    assert!(before.stats.error_mm.len() > 12 && before.stats.error_mm.iter().all(|v| *v == 0.0));
    h.ctl.lock().unwrap().sky = Some((18.0, 0.0, storm, 27));
    run_frames(&mut [&mut p], 1.0, drive);
    let brake = |_: usize, _: f64| (StepInput { brake: 1.0, ..Default::default() }, yaw);
    run_frames(&mut [&mut p], 2.0, brake);
    let end = p.pred.as_ref().unwrap();
    let wet = end.player.weather;
    assert_eq!((wet.rain_mm_h, wet.wind), (22.0, [11.5, 4.2]));
    assert!(wet.wetness > rain_state.wetness && wet.puddle_fill > rain_state.puddle_fill);
    assert!(end.stats.replayed_steps > 0 && end.stats.corrections >= 1);
    assert!(end.stats.error_mm.iter().rev().take(15).all(|v| *v == 0.0));
    println!("UDP rain to storm: {} acknowledgements, {} replayed inputs, {} corrections", end.stats.error_mm.len(), end.stats.replayed_steps, end.stats.corrections);
    let stats = h.stop();
    assert_eq!(stats["mismatches"], 0, "server disagreed after the authoritative weather change");
}

#[test]
fn drive_range_and_hill_hold_on_filbert_street_agree_over_udp() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let game = &world().0;
    // Choose the middle of an actual graded block, clear of the junction.
    let (grade, x, z, yaw) = game
        .city
        .roads
        .iter()
        .filter(|r| r.name == "Filbert Street" && !r.bridge && !r.tunnel)
        .flat_map(|r| r.points64.windows(2))
        .filter_map(|w| {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let len = dx.hypot(dz);
            if len < 32.0 {
                return None;
            }
            let (x, z) = ((w[1][0] + w[0][0]) * 0.5, (w[1][1] + w[0][1]) * 0.5);
            let (ux, uz) = (dx / len, dz / len);
            let grade = (game.ground.ground_y(x + 5.0 * ux, z + 5.0 * uz)
                - game.ground.ground_y(x - 5.0 * ux, z - 5.0 * uz))
                / 10.0;
            (0.08..0.20).contains(&grade.abs()).then_some((
                grade.abs(),
                x,
                z,
                (ux * grade.signum()).atan2(uz * grade.signum()),
            ))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .expect("graded Filbert Street block");
    let h = Harness::start();
    let mut p = Player::join(
        h.addr,
        "hill-driver",
        Conditions::NONE,
        [x, z, yaw],
        Some(VehicleConfig {
            model: VehicleModel::Sedan,
            ..Default::default()
        }),
    );
    wait_connected(&mut [&mut p]);
    let brake = |_: usize, _: f64| {
        (
            StepInput {
                brake: 1.0,
                ..Default::default()
            },
            yaw,
        )
    };
    run_frames(&mut [&mut p], 1.2, brake);
    let position = |p: &Player| {
        let s = p.pred.as_ref().unwrap();
        s.player.position(&s.cw)
    };
    let start = position(&p);
    let s = p.pred.as_ref().unwrap();
    let wheels = s.player.vehicles[0].controller.wheels();
    assert!(wheels.iter().all(|w| w.raycast_info().is_in_contact));
    let contact_grade = wheels
        .iter()
        .map(|w| {
            let n = w.raycast_info().contact_normal_ws;
            -(f64::from(n.x) * yaw.sin() + f64::from(n.z) * yaw.cos()) / f64::from(n.y)
        })
        .sum::<f64>()
        / wheels.len() as f64;
    assert!(
        contact_grade > 0.07,
        "wheel contacts on grade {contact_grade}"
    );
    p.frame(
        &StepInput {
            shift_direction: true,
            brake: 1.0,
            ..Default::default()
        },
        yaw,
        0.001,
    );
    run_frames(&mut [&mut p], 1.7, |_, _| {
        (
            StepInput {
                throttle: 0.6,
                ..Default::default()
            },
            yaw,
        )
    });
    let s = p.pred.as_ref().unwrap();
    assert!(
        s.player.vehicles[0].reverse_selected() && s.player.vehicles[0].forward_speed(&s.cw) < -2.0
    );
    let moved = position(&p) - start;
    assert!(moved.x * yaw.sin() + moved.z * yaw.cos() < -1.0);
    run_frames(&mut [&mut p], 2.0, brake);
    let stopped = position(&p);
    run_frames(&mut [&mut p], 2.0, brake);
    let drift = stopped.distance(position(&p));
    let s = p.pred.as_ref().unwrap();
    let samples = s.stats.error_mm.len();
    let peak_error = s.stats.error_mm.iter().copied().fold(0.0_f64, f64::max);
    let stats = h.stop();
    println!(
        "Filbert Street ({x:.2}, {z:.2}): terrain {grade:.3}, wheel grade {contact_grade:.3}, brake drift {drift:.4} m, {samples} acknowledgements, max error {peak_error} mm"
    );
    assert!(drift < 0.03 && samples > 30 && peak_error == 0.0);
    assert_eq!(stats["mismatches"], 0);
}

#[test]
fn ocean_beach_promenade_spawn_and_walk_agree_over_udp() {
    // Survey-plan coordinates at the sixth mapped stair opening.
    let [x, z] = eo_data::groundscale::plan([-8061.9855, 1143.8]);
    let yaw = 0.09829;
    let h = Harness::start();
    let mut p = Player::connect(h.addr, "beach-walker", Conditions::NONE, [x, z, yaw]);
    wait_connected(&mut [&mut p]);
    run_frames(&mut [&mut p], 0.75, |_, _| (StepInput::default(), yaw));
    let pred = p.pred.as_ref().unwrap();
    let start = pred.player.character.feet();
    assert!(pred.player.character.grounded);
    assert!(start.y - pred.cw.floor_y(start.x, start.z) > 0.25);
    run_frames(&mut [&mut p], 3.0, |_, _| {
        (StepInput { move_y: 1.0, ..Default::default() }, yaw)
    });
    let pred = p.pred.as_ref().unwrap();
    let end = pred.player.character.feet();
    let travel = (end.x - start.x) * yaw.sin() + (end.z - start.z) * yaw.cos();
    let height_error = (end.y - pred.cw.surface_y(end.x, end.z)).abs();
    let samples = pred.stats.error_mm.len();
    let max_error = pred.stats.error_mm.iter().copied().fold(0.0_f64, f64::max);
    let stats = h.stop();
    println!("Ocean Beach UDP: walked {travel:.2} m, floor gap {height_error:.4} m, {samples} acknowledgements, maximum prediction error {max_error:.3} mm");
    assert!(travel > 3.0 && height_error < 0.04 && samples > 20 && max_error == 0.0);
    assert_eq!(stats["mismatches"], 0);
}
