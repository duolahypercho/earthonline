//! The server and a client replaying the same inputs arrive at the same
//! player, bit for bit: the server's world is fed from the shared tile set,
//! the client's cooks its own tiles on its own threads (different insertion
//! order, different collider handles). And a client that rewinds to a state
//! the server sent and replays its inputs arrives where it had been.

use eo_net::game::{GameWorld, rebuild_player, spawn_player, step_player};
use eo_net::wire::{NetInput, read_player_state, write_player_state};
use eo_player::Mode;
use eo_player::world::{CollisionWorld, GroundProvider, SurfaceMesh, TileKey};
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
fn base_bridge_and_pier_floors_catch_server_and_predicted_players() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let g = GameWorld::load_access(&paths, false, None, false).unwrap();
    let deck = eo_net::game::base_bridge_walk(&g.elevation);
    let mid = deck.walk[deck.walk.len()/2].into_iter().map(DVec3::from_array).sum::<DVec3>()/3.;
    let [tip, z] = eo_data::groundscale::plan([-431.5, -3610.0]);
    let pier = DVec3::new(tip - 6., 35., z);
    let mut reference = game().collision_world(1);
    reference.load_blocking(pier, 32.);
    let top = reference.raycast(pier, DVec3::NEG_Y, 60., eo_player::world::layer::STREET, None).unwrap().point;
    assert!(top.y > reference.floor_y(top.x, top.z) + 0.1);
    let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 2);
    for (place, floor) in [("Golden Gate east sidewalk", mid), ("Municipal Pier", top)] {
        let start = floor + DVec3::Y * 0.6;
        let mut server_cw = CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles.clone());
        let mut predicted_cw = g.collision_world(1);
        let mut server = eo_player::Player::new(start, 0.);
        let mut predicted = eo_player::Player::new(start, 0.);
        for cw in [&mut server_cw, &mut predicted_cw] {
            cw.load_blocking(start, eo_player::world::LOAD_RADIUS_M);
        }
        for tick in 0..120 {
            step_player(&mut server, &mut server_cw, &NetInput::default());
            step_player(&mut predicted, &mut predicted_cw, &NetInput::default());
            assert_eq!(server.state(&server_cw), predicted.state(&predicted_cw), "{place}, tick {tick}");
        }
        let feet = server.character.feet();
        eprintln!("{place}: drawn floor {floor:.3?}, network feet {feet:.3?}");
        assert!(server.character.grounded && (feet.y-floor.y).abs() < 0.05, "{place} lost the floor");
    }
}

#[test]
fn bridge_traffic_lanes_carry_the_same_loaded_van_on_server_and_client() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    use eo_player::{Player, StepInput};
    let paths = eo_data::DataPaths::discover().unwrap();
    for structured in [false, true] {
        let options = structured.then_some(eo_structures::StructureOptions { tunnels: false });
        let g = GameWorld::load_access(&paths, true, options, false).unwrap();
        let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 2);
        for deck in eo_net::game::bridge_roads(&paths, &g.elevation).unwrap() {
            let mid = (deck.walk.len() / 4) * 2;
            let [p, q, r] = deck.walk[mid].map(DVec3::from_array);
            let end = DVec3::from_array(deck.walk[mid+1][2]).lerp(r, 0.75);
            let start = p.lerp(q, 0.75);
            let heading = end-start;
            let yaw = heading.x.atan2(heading.z);
            let mut server_cw = CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles.clone());
            server_cw.load_blocking(start, eo_player::world::LOAD_RADIUS_M);
            let mut server = Player::new(start, yaw);
            server.spawn_config = VehicleConfig { model: VehicleModel::DeliveryVan, cargo_kg: 600, occupants_kg: 80 };
            let i = server.spawn_vehicle_below(&mut server_cw, start + DVec3::Y*0.3, yaw);
            server.enter_vehicle(i);
            let mut bytes = Vec::new();
            write_player_state(&server.state(&server_cw), &mut bytes);
            let mut client_cw = g.collision_world(2);
            let mut client = rebuild_player(&mut client_cw, &read_player_state(&bytes).unwrap(),
                &[[start.x as f32, start.z as f32, yaw as f32]]);
            for tick in 0..1200 {
                let step = if tick < 120 { StepInput { handbrake: true, ..Default::default() } }
                    else { StepInput { throttle: 0.65, ..Default::default() } };
                let input = NetInput::quantize(&step, yaw);
                step_player(&mut server, &mut server_cw, &input);
                step_player(&mut client, &mut client_cw, &input);
                assert_eq!(server.state(&server_cw), client.state(&client_cw), "{} tick {tick}", deck.name);
                if tick >= 120 {
                    assert_eq!(server.vehicles[i].wheels_in_contact(),4,"{} tick {tick}",deck.name);
                }
            }
            let travelled = client.position(&client_cw).distance(start);
            println!("network {}, structures={structured}: 1200 identical ticks, {travelled:.2} m",deck.name);
            assert!(travelled > 100. && (client.position(&client_cw).y - start.y).abs() < 10.);
        }
    }
}

/// Every landmark lift through the server's step: a client stands at the
/// bottom stop, taps interact, rides up, walks the deck, comes back to the
/// stop and rides down; the server runs the same quantised inputs in its fed
/// world (the shared tiles, the same decks) and the two agree on every step.
#[test]
fn every_landmark_lift_through_the_server_agrees_bit_for_bit() {
    use eo_player::StepInput;
    use eo_player::lift::RIDE_STEPS;
    let g = game();
    let lifts = g.buildings.decks.lifts.clone();
    assert!(lifts.len() >= 7, "the server's world has {} lifts", lifts.len());
    let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 3);
    for l in &lifts {
        // Both ends on foot at the stop with no car: a parked car settling on
        // uneven ground (the Bay Bridge entrance is on the quay's edge) has no
        // contact history in a freshly rebuilt world and drifts apart by
        // 1e-5 m (the cross-world car defect the known rewind test records);
        // the ride is the subject here.
        let feet = glam::DVec3::new(l.bottom.x, l.bottom.y.max(g.ground.ground_y(l.bottom.x, l.bottom.z)) + 0.05, l.bottom.z);
        let mut server_cw = CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles.clone());
        server_cw.load_blocking(feet, eo_player::world::LOAD_RADIUS_M);
        let mut server = eo_player::Player::new(feet, l.bottom_yaw);
        let mut bytes = vec![];
        write_player_state(&server.state(&server_cw), &mut bytes);
        let mut client_cw = g.collision_world(2);
        let mut client = rebuild_player(&mut client_cw, &read_player_state(&bytes).unwrap(), &[]);
        assert_eq!(client.state(&client_cw), server.state(&server_cw), "{}: the client starts where the server is", l.name);
        let ride = RIDE_STEPS as usize;
        let script: Vec<(usize, StepInput)> = vec![
            (30, StepInput::default()),
            (1, StepInput { interact: true, ..Default::default() }),
            (ride + 30, StepInput::default()),
            (40, StepInput { move_y: 1.0, ..Default::default() }),
            (40, StepInput { move_y: -1.0, ..Default::default() }),
            (40, StepInput::default()),
            (1, StepInput { interact: true, ..Default::default() }),
            (ride + 60, StepInput::default()),
        ];
        let (mut was_up, mut k) = (false, 0);
        for (n, input) in script {
            for _ in 0..n {
                let i = NetInput::quantize(&input, client.yaw);
                step_player(&mut server, &mut server_cw, &i);
                step_player(&mut client, &mut client_cw, &i);
                server_cw.pump(server.position(&server_cw));
                client_cw.pump(client.position(&client_cw));
                assert_eq!(server.state(&server_cw), client.state(&client_cw), "{}: step {k}: the client's replay differs from the server", l.name);
                let f = server.position(&server_cw);
                was_up |= server.lift.is_none() && (f - l.top).length() < 2.5;
                k += 1;
            }
        }
        let end = server.position(&server_cw);
        assert!(was_up, "{}: the server never had the player on the deck", l.name);
        assert!((end.x - l.bottom.x).hypot(end.z - l.bottom.z) < 1.0 && (end.y - l.bottom.y).abs() < 4.0, "{}: not back at the entrance: {end:?} (stop {:?})", l.name, l.bottom);
        println!("{}: up and down through the server, bit for bit over {k} steps", l.name);
    }
}

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

/// The city's ground with every tile's street surface `delay_ms` slower to
/// cook: a loaded machine, a debug build or a slower CPU, where the replays
/// above (which step far faster than real time) outrun the tile cookers.
struct SlowCook(Arc<dyn GroundProvider>, u64);

impl GroundProvider for SlowCook {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.0.ground_y(x, z)
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        std::thread::sleep(std::time::Duration::from_millis(self.1));
        self.0.surface_tile(key)
    }
    // (the wrapped ground's levels and cuts: the same world, only slower)
    fn arrival_road_y(&self, x: f64, z: f64, near_y: f64) -> Option<f64> {
        self.0.arrival_road_y(x, z, near_y)
    }
    fn multi_level(&self) -> bool {
        self.0.multi_level()
    }
    fn terrain_cuts_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<&eo_structures::TerrainCut> {
        self.0.terrain_cuts_in(min, max)
    }
    fn terrain_cut(&self, x: f64, z: f64) -> Option<f64> {
        self.0.terrain_cut(x, z)
    }
}

/// The nondeterminism behind the rewind test's failures: before every
/// authoritative step had the tiles it reaches made resident, a run's
/// colliders at a step depended on how fast the worker threads had cooked,
/// and a body stepped faster than real time ran over ground that was not
/// there yet (a 40 ms cook: 1157 of 2400 steps over a missing tile, replays
/// parted by 4 to 1850 m). Now the same inputs give the same body, step for
/// step, with fast and slow cooks, and the fresh-world replays of a
/// slow-cooking world land where the uninterrupted run did.
#[test]
fn a_slow_tile_cook_does_not_change_a_single_step() {
    let g = game();
    let slow = GameWorld::from_parts(g.city.clone(), g.elevation.clone(), Arc::new(SlowCook(g.ground.clone(), 40)), g.buildings.clone(), g.streets, g.structures);
    let (route, spawn) = script_route(g);
    let mut fast_cw = g.collision_world(2);
    let (mut fast, cars) = spawn_player(&mut fast_cw, spawn[0], spawn[1], spawn[2], false);
    let mut slow_cw = slow.collision_world(2);
    let (mut lagging, _) = spawn_player(&mut slow_cw, spawn[0], spawn[1], spawn[2], false);
    let mut script = Script::new(route, [cars[0][0] as f64, cars[0][1] as f64]);
    let mut inputs = vec![];
    let mut saved: Vec<(usize, Vec<u8>)> = vec![];
    for k in 0..STEPS {
        let i = script.input(k, STEPS, &mut fast, &fast_cw);
        inputs.push(i);
        step_player(&mut fast, &mut fast_cw, &i);
        step_player(&mut lagging, &mut slow_cw, &i);
        fast_cw.pump(fast.position(&fast_cw));
        slow_cw.pump(lagging.position(&slow_cw));
        assert!(slow_cw.is_resident(lagging.position(&slow_cw).x, lagging.position(&slow_cw).z), "step {k}: the body stands on a tile that is not resident");
        assert_eq!(fast.state(&fast_cw), lagging.state(&slow_cw), "step {k}: a slow cook changed the step");
        if k % 600 == 300 {
            let mut b = vec![];
            write_player_state(&lagging.state(&slow_cw), &mut b);
            saved.push((k + 1, b));
        }
    }
    println!("slow cook: {} steps waited for a tile, the longest {:.1} ms", slow_cw.step_waits, slow_cw.step_wait_ms_max);
    let end = lagging.state(&slow_cw);
    for (k, bytes) in &saved {
        let mut cw2 = slow.collision_world(2);
        let mut p2 = rebuild_player(&mut cw2, &read_player_state(bytes).unwrap(), &cars);
        for i in &inputs[*k..] {
            step_player(&mut p2, &mut cw2, i);
            cw2.pump(p2.position(&cw2));
        }
        let d = (DVec3::from_array(p2.state(&cw2).character.position) - DVec3::from_array(end.character.position)).length();
        println!("slow cook: rewound to step {k} in a fresh world, replayed {} inputs: {d:.5} m from the uninterrupted run", STEPS - k);
        assert!(d < 0.10, "slow cook: the fresh-world replay from step {k} ends {d:.4} m away");
    }
}

#[test]
fn a_loaded_van_rebuilds_and_drives_identically_on_both_peers() {
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let game = game();
    let (_, spawn) = script_route(game);
    let mut server_world = game.collision_world(1);
    let config = VehicleConfig { model: VehicleModel::DeliveryVan, cargo_kg: 420, occupants_kg: 82 };
    let (mut server, cars) = eo_net::game::spawn_configured_player(&mut server_world, spawn[0], spawn[1], spawn[2], true, config);
    assert!(matches!(server.mode, Mode::Driving(_)));
    let mut bytes = Vec::new();
    write_player_state(&server.state(&server_world), &mut bytes);
    let mut client_world = game.collision_world(2);
    let mut client = rebuild_player(&mut client_world, &read_player_state(&bytes).unwrap(), &cars);
    assert_eq!(client.vehicles[0].spec.config, config);
    let start = server.position(&server_world);
    for tick in 0..240 {
        let step = eo_player::StepInput { throttle: 0.6, steer: if tick > 180 { 0.12 } else { 0.0 }, ..Default::default() };
        let input = NetInput::quantize(&step, spawn[2]);
        step_player(&mut server, &mut server_world, &input);
        step_player(&mut client, &mut client_world, &input);
        assert_eq!(client.state(&client_world), server.state(&server_world), "diverged on step {tick}");
    }
    assert!(server.position(&server_world).distance(start) > 3.0);
    let arrival = eo_net::game::arrival_state(&server, &mut server_world, spawn[0] + 20.0, spawn[1], spawn[2]);
    assert_eq!(arrival.vehicles[0].config, config);
}

#[test]
fn dedicated_players_stand_on_the_municipal_pier_and_marina_dock_surfaces() {
    let game = game();
    let part = game.buildings.decks.parts.iter().find(|p| p.name == "harbour piers and docks").unwrap();
    let [tip,z] = eo_data::groundscale::plan([-431.5,-3610.0]);
    let x = tip - 6.0;
    let mut cw = game.collision_world(1);
    cw.load_blocking(DVec3::new(x,0.0,z),64.0);
    let y = cw.surface_below(x,z,12.0);
    assert!(y > cw.floor_y(x,z)+0.1);
    // The final triangles are the actual guide-pile dock's walking strip.
    let tri = part.walk.last().unwrap();
    let dock = (DVec3::from_array(tri[0])+DVec3::from_array(tri[1])+DVec3::from_array(tri[2]))/3.0;
    for at in [DVec3::new(x,y,z),dock] {
        cw.load_blocking(at,64.0);
        let mut p = eo_player::Player::new(at + DVec3::Y * 0.35,0.0);
        for _ in 0..120 { eo_net::game::step_player(&mut p,&mut cw,&NetInput::default()); }
        assert!(p.character.grounded && (p.position(&cw).y-at.y).abs() < 0.04,"{at:?} -> {:?}",p.position(&cw));
    }
}

#[test]
fn ocean_beach_landing_panel_seams_and_checkpoint_match_the_authoritative_world() {
    let g = game();
    let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 2);
    let mut server_world =
        CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles.clone());
    let mut client_world = g.collision_world(1);
    // The middle of the promenade at its sixth mapped flight. A play-mode
    // regression independently measures the corresponding rendered panels.
    let [x, z] = eo_data::groundscale::plan([-8061.9855, 1143.8]);
    let yaw = 0.09829;
    let near = DVec3::new(x, g.ground.ground_y(x, z) + 2.0, z);
    server_world.load_blocking(near, 32.0);
    let start_y = server_world.surface_below(x, z, near.y);
    assert!(start_y - server_world.floor_y(x, z) > 0.25);
    let feet = DVec3::new(x, start_y + 0.65, z);
    let mut server = eo_player::Player::new(feet, yaw);
    let mut bytes = Vec::new();
    write_player_state(&server.state(&server_world), &mut bytes);
    let mut client = rebuild_player(&mut client_world, &read_player_state(&bytes).unwrap(), &[]);
    let mut max_drop = 0.0_f64;
    for tick in 0..600 {
        let input = eo_player::StepInput {
            move_y: if tick >= 120 { 1.0 } else { 0.0 },
            jump: tick == 327,
            ..Default::default()
        };
        let command = NetInput::quantize(&input, yaw);
        step_player(&mut server, &mut server_world, &command);
        step_player(&mut client, &mut client_world, &command);
        let pos = server.character.feet();
        let floor = server_world.surface_below(pos.x, pos.z, pos.y + 0.25);
        max_drop = max_drop.max(floor - pos.y);
        assert_eq!(
            client.state(&client_world),
            server.state(&server_world),
            "Ocean Beach tick {tick}"
        );
        if tick == 299 {
            bytes.clear();
            write_player_state(&server.state(&server_world), &mut bytes);
            client_world = g.collision_world(1);
            client = rebuild_player(&mut client_world, &read_player_state(&bytes).unwrap(), &[]);
        }
    }
    let end = server.character.feet();
    println!(
        "Ocean Beach: 600 authoritative ticks, restore at 300, walked {:.2} m, maximum floor penetration {max_drop:.4} m",
        (end.x - x).hypot(end.z - z)
    );
    assert!(server.character.grounded && (end.x - x).hypot(end.z - z) > 10.0 && max_drop < 0.035);
    // Cloning a pristine collider retains its immutable, potentially large
    // trimesh and spatial index. A new player must not re-cook this map.
    let seaside = g
        .buildings
        .decks
        .parts
        .iter()
        .find(|p| p.name == "Ocean Beach Esplanade")
        .unwrap();
    let shape = |cw: &CollisionWorld| {
        cw.physics
            .colliders
            .iter()
            .find_map(|(_, collider)| {
                collider
                    .shape()
                    .as_trimesh()
                    .filter(|m| m.indices().len() == seaside.walk.len())
                    .map(|_| collider.shared_shape().clone())
            })
            .unwrap()
    };
    assert!(Arc::ptr_eq(
        &shape(&client_world).0,
        &shape(&server_world).0
    ));
}

#[test]
fn loading_collision_between_inputs_preserves_a_storm_driven_free_flight_and_landing() {
    use eo_net::game::{spawn_configured_player, step_player_in_weather};
    use eo_net::weather::{RoadWeather, WeatherEpoch};
    use eo_player::vehicle::{VehicleConfig, VehicleModel};
    let g = game();
    let tiles = SharedTiles::start(g.ground.clone(), g.buildings.clone(), 2);
    let mut server_world = CollisionWorld::fed(g.ground.clone(), g.buildings.clone(), tiles);
    let [x, z] = eo_data::groundscale::plan([84.6480, -2459.4888]); // Filbert Street
    let yaw = 0.988_f64.atan2(-0.156);
    let (mut server, cars) = spawn_configured_player(
        &mut server_world,
        x,
        z,
        yaw,
        false,
        VehicleConfig {
            model: VehicleModel::Sedan,
            ..Default::default()
        },
    );
    let body = server.vehicles[0].body;
    let mut elevated = server_world.physics.bodies[body].translation();
    elevated.y += 4.0;
    server_world.physics.bodies[body].set_translation(elevated, true);
    server_world.physics.bodies[body].set_linvel([5.0, 2.0, 0.0].into(), true);
    let mut wire = Vec::new();
    write_player_state(&server.state(&server_world), &mut wire);
    let mut client_world = g.collision_world(1);
    let mut client = rebuild_player(&mut client_world, &read_player_state(&wire).unwrap(), &cars);
    let storm = WeatherEpoch::new(
        eo_net::presence::weather_code("storm").unwrap(),
        123,
        45_000,
        0,
    );
    let mut server_weather = RoadWeather::new(storm);
    let mut client_weather = RoadWeather::new(storm);
    let input = NetInput::quantize(&Default::default(), yaw);
    let mut free = 0;
    for seq in 1..=180 {
        step_player_in_weather(
            &mut server,
            &mut server_world,
            &input,
            seq,
            &mut server_weather,
        );
        step_player_in_weather(
            &mut client,
            &mut client_world,
            &input,
            seq,
            &mut client_weather,
        );
        if seq == 20 {
            assert_eq!(server.vehicles[0].wheels_in_contact(), 0);
            let at = server.vehicles[0].datum(&server_world);
            server_world.load_blocking(at + DVec3::Z * 256.0, 32.0);
        }
        if server.vehicles[0].wheels_in_contact() == 0 {
            free += 1;
        }
        assert_eq!(
            server.state(&server_world),
            client.state(&client_world),
            "Filbert Street storm tick {seq}"
        );
    }
    assert_eq!(server.mode, Mode::OnFoot);
    assert!(free >= 45 && server.vehicles[0].wheels_in_contact() == 4);
    println!(
        "Unoccupied sedan on Filbert Street in a storm: {free} unsupported ticks, 180 matching authoritative states, four wheels down after landing"
    );
}

/// Fast travel lands beside a low covered parking space. The neighbouring
/// roof must neither claim the parked car nor lift the driver when exiting.
#[test]
fn a_covered_parking_space_keeps_arrival_exit_and_jump_on_the_same_floor() {
    use eo_player::{Player, StepInput, world::{BuildingSet, DeckPart, Decks, LOAD_RADIUS_M}};
    struct Flat;
    impl GroundProvider for Flat { fn ground_y(&self, _: f64, _: f64) -> f64 { 0. } }
    let ground: Arc<dyn GroundProvider> = Arc::new(Flat);
    let roof = [[-8., 1.98, -5.], [-8., 1.98, 5.], [0., 1.98, 5.], [0., 1.98, -5.]];
    let buildings = Arc::new(BuildingSet::empty().with_decks(Decks {
        parts: vec![DeckPart { name: "covered parking".into(), walk: vec![
            [roof[0], roof[1], roof[2]], [roof[0], roof[2], roof[3]],
        ], ..Default::default() }], ..Default::default()
    }));
    let tiles = SharedTiles::start(ground.clone(), buildings.clone(), 2);
    let mut server_world = CollisionWorld::fed(ground.clone(), buildings.clone(), tiles);
    server_world.load_blocking(DVec3::ZERO, LOAD_RADIUS_M);
    let mut server = Player::new(DVec3::X, 0.);
    server.spawn_vehicle(&mut server_world, 3., 0., 0.);
    let arrival = eo_net::game::arrival_state(&server, &mut server_world, 1., 0., 0.);
    server.set_state(&mut server_world, &arrival);
    assert!(server.vehicles[0].datum(&server_world).y.abs() < 0.01, "arrival parked on the roof");
    let mut bytes = vec![];
    write_player_state(&arrival, &mut bytes);
    let mut client_world = CollisionWorld::new(ground, buildings, 1);
    let mut client = rebuild_player(&mut client_world, &read_player_state(&bytes).unwrap(), &[[-3., 0., 0.]]);
    // Walk from outside under the roof, enter the car, exit, then jump.
    let mut hit_ceiling = false;
    for tick in 0..160 {
        let cmd = StepInput { move_x: if tick < 40 { 1. } else { 0. },
            vehicle_toggle: matches!(tick, 75 | 85), jump: tick == 100, ..Default::default() };
        let input = NetInput::quantize(&cmd, 0.);
        step_player(&mut server, &mut server_world, &input);
        step_player(&mut client, &mut client_world, &input);
        assert_eq!(server.state(&server_world), client.state(&client_world), "covered parking tick {tick}");
        if tick == 75 { assert_eq!(server.mode, Mode::Driving(0), "failed to enter from {:?}", server.character.feet()); }
        if tick == 85 {
            assert_eq!(server.mode, Mode::OnFoot);
            assert!(server.character.feet().y.abs() < 0.03 && server.character.feet().x < 0., "exit missed the covered space");
        }
        if tick > 100 {
            let head = server.character.position.y + server.character.half_extent_y();
            assert!(head <= 1.981, "head through roof at {head}");
            hit_ceiling |= (1.95..=1.981).contains(&head) && server.character.vertical_velocity.abs() < 0.001;
        }
    }
    assert!(hit_ceiling && server.character.grounded, "jump must strike the roof and land");
}
