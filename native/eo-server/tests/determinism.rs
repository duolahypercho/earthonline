//! The server and a client replaying the same inputs arrive at the same
//! player, bit for bit: the server's world is fed from the shared tile set,
//! the client's cooks its own tiles on its own threads (different insertion
//! order, different collider handles). And a client that rewinds to a state
//! the server sent and replays its inputs arrives where it had been.

use eo_net::game::{GameWorld, rebuild_player, spawn_player, step_player};
use eo_net::wire::{NetInput, read_player_state, write_player_state};
use eo_player::Mode;
use eo_player::world::CollisionWorld;
use eo_server::predict::{Script, script_route};
use eo_server::tiles::SharedTiles;
use glam::DVec3;
use std::sync::{Arc, OnceLock};

fn game() -> &'static Arc<GameWorld> {
    static G: OnceLock<Arc<GameWorld>> = OnceLock::new();
    G.get_or_init(|| Arc::new(GameWorld::load(&eo_data::DataPaths::discover().unwrap(), true).unwrap()))
}

const STEPS: usize = 2400;

#[test]
fn server_and_client_agree_bit_for_bit_walking_and_driving() {
    let g = game();
    let (route, spawn) = script_route(g);
    let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 3);
    let mut server_cw = CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles.clone());
    let (mut server, cars) = spawn_player(&mut server_cw, spawn[0], spawn[1], spawn[2], false);
    let mut state = vec![];
    write_player_state(&server.state(&server_cw), &mut state);
    let mut client_cw = g.collision_world(2);
    let mut client = rebuild_player(&mut client_cw, &read_player_state(&state).unwrap(), &cars);
    assert_eq!(client.state(&client_cw), server.state(&server_cw), "the client starts where the server is");
    let mut script = Script::new(route, [cars[0][0] as f64, cars[0][1] as f64]);
    let (mut drove, mut walked, mut entered, mut left) = (0, 0, 0, 0);
    let mut travelled = 0.0;
    let mut last = server.position(&server_cw);
    for k in 0..STEPS {
        // The client decides the input (its pilot reads its own body), the server runs it.
        let i: NetInput = script.input(k, STEPS, &mut client, &client_cw);
        let before = server.mode;
        step_player(&mut server, &mut server_cw, &i);
        step_player(&mut client, &mut client_cw, &i);
        server_cw.pump(server.position(&server_cw));
        client_cw.pump(client.position(&client_cw));
        assert_eq!(server.state(&server_cw), client.state(&client_cw), "step {k}: the client's replay differs from the server");
        match (before, server.mode) {
            (Mode::OnFoot, Mode::Driving(_)) => entered += 1,
            (Mode::Driving(_), Mode::OnFoot) => left += 1,
            _ => {}
        }
        match server.mode {
            Mode::Driving(_) => drove += 1,
            Mode::OnFoot => walked += 1,
        }
        let p = server.position(&server_cw);
        travelled += (p - last).length();
        last = p;
    }
    println!("bit-for-bit over {STEPS} steps: walked {walked}, drove {drove}, got in {entered}, got out {left}, {travelled:.0} m");
    assert!(entered == 1 && left == 1, "got in {entered} and out {left} times");
    assert!(drove > 900 && walked > 600, "the script walked {walked} and drove {drove} steps");
    assert!(travelled > 200.0, "travelled {travelled} m");
}

#[test]
fn a_client_rewound_to_a_server_state_replays_to_the_same_place() {
    let g = game();
    let (route, spawn) = script_route(g);
    let mut cw = g.collision_world(2);
    let (mut player, cars) = spawn_player(&mut cw, spawn[0], spawn[1], spawn[2], false);
    let mut script = Script::new(route, [cars[0][0] as f64, cars[0][1] as f64]);
    let mut saved: Vec<(usize, Vec<u8>)> = vec![];
    let mut inputs: Vec<NetInput> = vec![];
    for k in 0..STEPS {
        let i = script.input(k, STEPS, &mut player, &cw);
        inputs.push(i);
        step_player(&mut player, &mut cw, &i);
        cw.pump(player.position(&cw));
        if k % 300 == 150 {
            let mut b = vec![];
            write_player_state(&player.state(&cw), &mut b);
            saved.push((k + 1, b));
        }
    }
    let end = player.state(&cw);
    // In place (a correction): back 40 steps, replay, land on the same body.
    {
        let k0 = STEPS - 40;
        let mut cw3 = g.collision_world(2);
        let (mut p3, _) = spawn_player(&mut cw3, spawn[0], spawn[1], spawn[2], false);
        let mut mid = vec![];
        for (k, i) in inputs.iter().enumerate() {
            step_player(&mut p3, &mut cw3, i);
            cw3.pump(p3.position(&cw3));
            if k + 1 == k0 {
                write_player_state(&p3.state(&cw3), &mut mid);
            }
        }
        let end3 = p3.state(&cw3);
        assert_eq!(end3, end, "the same inputs from the same spawn");
        p3.set_state(&mut cw3, &read_player_state(&mid).unwrap());
        for i in &inputs[k0..] {
            step_player(&mut p3, &mut cw3, i);
        }
        assert_eq!(p3.state(&cw3), end, "rewound in place 40 steps and replayed");
    }
    // In a fresh world (a reconnect): every saved state, replayed to the end.
    let mut worst: (f64, String) = (0.0, String::new());
    for (k, bytes) in &saved {
        let s = read_player_state(bytes).unwrap();
        let mut cw2 = g.collision_world(2);
        let mut p2 = rebuild_player(&mut cw2, &s, &cars);
        for i in &inputs[*k..] {
            step_player(&mut p2, &mut cw2, i);
            cw2.pump(p2.position(&cw2));
        }
        let d = (DVec3::from_array(p2.state(&cw2).character.position) - DVec3::from_array(end.character.position)).length();
        let what = if s.driving.is_some() { "driving" } else { "on foot" };
        println!("rewound to step {k} ({what}) in a fresh world, replayed {} inputs: {d:.5} m from the uninterrupted run", STEPS - k);
        if d > worst.0 {
            worst = (d, format!("step {k} ({what})"));
        }
    }
    // A fresh world has no contact history for the car's chassis, so a replay
    // through a stretch of driving may differ by centimetres; the position
    // check corrects it. On foot nothing is hidden.
    assert!(worst.0 < 0.10, "worst fresh-world replay {:.4} m at {}", worst.0, worst.1);
}
