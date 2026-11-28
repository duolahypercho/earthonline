//! The world as the game runs it: the shipped defaults (42 cars, People
//! `medium`) around a moving focus on a natively built network, determinism
//! (same seed, same result, on one thread or many), tile streaming across the
//! city, and the instance buffers.

use eo_sim::native::{self, NativeSource};
use eo_sim::output::{AgentInstance, KIND_WALKER_BASE};
use eo_sim::region::{Focus, Region, RegionConfig};
use eo_sim::{Network, World};
use std::sync::OnceLock;

fn source() -> &'static NativeSource {
    static S: OnceLock<NativeSource> = OnceLock::new();
    S.get_or_init(|| native::load(&eo_data::DataPaths::discover().unwrap()).unwrap())
}

/// SoMa on the shipped (scaled) plan: the bench spawn on Folsom Street's
/// walk (the old point, (1596.6, -318.4) surveyed, is mid-block: on the 1.25 x plan it
/// stands 62 m from the nearest footway, past the 55 m people disc, where no player
/// ever stands).
static SOMA: std::sync::LazyLock<[f64; 2]> = std::sync::LazyLock::new(|| eo_data::groundscale::plan([1539.0, -366.0]));

fn focus_at(c: [f64; 2], t: f64) -> Focus {
    let a = t * 1.4 / 40.0;
    Focus { x: c[0] + 40.0 * a.cos(), z: c[1] + 40.0 * a.sin(), yaw: a + std::f64::consts::FRAC_PI_2 }
}

fn world_around(centres: &[[f64; 2]], threads: usize) -> World {
    let src = source();
    let mut tiles = vec![];
    for c in centres {
        tiles.extend(native::tiles_around(c[0], c[1], 600.0));
    }
    tiles.sort();
    tiles.dedup();
    let mut net = Network::new(src.timing.clone());
    net.add(src.build(&tiles));
    let mut world = World::new(net);
    for (k, c) in centres.iter().enumerate() {
        let r = Region::new(&world.net, k as u32, RegionConfig::game_default(1), focus_at(*c, 0.0));
        world.regions.push(r);
    }
    world.set_threads(threads);
    world
}

fn run(world: &mut World, centres: &[[f64; 2]], steps: usize) {
    for s in 0..steps {
        for (k, c) in centres.iter().enumerate() {
            world.set_focus(k, focus_at(*c, s as f64 / 60.0));
        }
        world.step();
    }
}

#[test]
fn game_defaults_hold_their_population_and_rules() {
    let mut world = world_around(&[*SOMA], 0);
    run(&mut world, &[*SOMA], 120 * 60);
    let r = &world.regions[0];
    let f = Focus { ..r.focus };
    assert_eq!(r.fleet.cars.len(), 42, "the moving pool");
    assert!(r.crowd.walkers.len() >= 30 && r.crowd.walkers.len() <= 40, "the medium disc holds 40 bodies, has {}", r.crowd.walkers.len());
    for (i, _) in r.crowd.walkers.iter().enumerate() {
        let d = (r.crowd.x[i] - f.x).hypot(r.crowd.z[i] - f.z);
        assert!(d <= 71.0 + 1.0, "walker {i} beyond the keep radius: {d}");
    }
    let near = (0..r.fleet.cars.len()).filter(|&i| (r.fleet.x[i] - f.x).hypot(r.fleet.z[i] - f.z) <= 120.0).count();
    let moving = r.fleet.cars.iter().filter(|c| c.speed > 0.5).count();
    eprintln!("after 120 s: {near} cars within 120 m, {moving} moving, {} walkers; recycles {}; {:?}", r.crowd.walkers.len(), r.stats.car_recycles, r.fleet.stats);
    assert!(near >= 10, "the recycler keeps the fleet near the focus ({near} within 120 m)");
    assert!(moving >= 8, "traffic flows ({moving} of 42 moving)");
    assert_eq!(r.fleet.stats.red_entries, 0, "no car enters on red from behind its bar");
    assert_eq!(r.crowd.stats.starts_outside_walk, 0, "no walker steps off outside WALK");
    let s = eo_sim::stats::sample(&world.net, r, world.time);
    assert_eq!(s.off_lane, 0);
    assert_eq!(s.overlaps, 0, "car bodies overlap");
    // The buffers: one record per agent, finite, kinds in range, near the region origin.
    let out = &r.out;
    assert_eq!(out.vehicles.len(), 42);
    assert_eq!(out.walkers.len(), r.crowd.walkers.len());
    let ok = |a: &AgentInstance| a.pos.iter().all(|v| v.is_finite() && v.abs() < 2000.0) && a.heading.is_finite() && a.speed >= 0.0;
    assert!(out.vehicles.iter().all(|a| ok(a) && a.kind < 4));
    assert!(out.walkers.iter().all(|a| ok(a) && a.kind >= KIND_WALKER_BASE && a.kind < KIND_WALKER_BASE + 7));
    assert_eq!(bytemuck::cast_slice::<AgentInstance, u8>(&out.vehicles).len(), 42 * 32);
}

#[test]
fn a_traffic_trip_runs_from_the_golden_gate_deck_to_its_approach() {
    let paths = eo_data::DataPaths::discover().unwrap();
    // Southbound Golden Gate Bridge in the shipped extract.
    let way = "537838948";
    let raw = eo_data::sim_source::SimSource::load(&paths).unwrap();
    let road = raw.roads.iter().find(|r| r.id == way).unwrap();
    let [x, z] = *road.points.last().unwrap();
    let [mid_x, mid_z] = road.points[road.points.len() / 2];
    let source = native::load(&paths).unwrap().with_levels([(way.into(), 1)].into());
    let mid_tiles = source.resident_tiles(mid_x, mid_z, 120.0);
    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&mid_tiles));
    assert!(net.alive_lanes().any(|l| net.lanes[l as usize].key.starts_with("n537838948:")));

    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&source.resident_tiles(x, z, 600.0)));
    let (from, link, to) = net.alive_lanes().find_map(|id| {
        let lane = &net.lanes[id as usize];
        if !lane.key.starts_with("n537838948:") {
            return None;
        }
        lane.links.iter().enumerate().find_map(|(i, l)| {
            (l.legal && l.drivable && net.lanes[l.to as usize].level == 0).then_some((id, i, l.to))
        })
    }).expect("southbound deck connects to its mapped approach");
    let mut world = World::new(net);
    let mut cfg = RegionConfig::js_pool(1, 0);
    cfg.vehicle_pool = 1;
    let mut region = Region::new(&world.net, 0, cfg, Focus { x, z, yaw: 0.0 });
    let length = world.net.lanes[from as usize].length;
    region.fleet.place(&world.net, 0, from, length - 60.0, 6.0);
    region.fleet.cars[0].next = link as u16;
    let id = region.fleet.cars[0].id;
    let mut prev = [region.fleet.x[0], region.fleet.z[0]];
    world.regions.push(region);
    for _ in 0..1200 {
        world.step();
        let f = &world.regions[0].fleet;
        let pos = [f.x[0], f.z[0]];
        assert_eq!(f.cars[0].id, id);
        assert!((pos[0] - prev[0]).hypot(pos[1] - prev[1]) < 1.0, "jump leaving the bridge");
        if f.cars[0].lane == to && !f.cars[0].cornering() {
            return;
        }
        prev = pos;
    }
    panic!("the vehicle did not reach the approach");
}

fn fingerprint(world: &World) -> Vec<u64> {
    let mut v = vec![];
    for r in &world.regions {
        for a in r.out.vehicles.iter().chain(&r.out.walkers) {
            v.push(a.pos[0].to_bits() as u64 ^ ((a.pos[2].to_bits() as u64) << 32));
            v.push(a.heading.to_bits() as u64 ^ ((a.id as u64) << 32));
        }
    }
    v
}

#[test]
fn same_seed_same_world_on_any_thread_count() {
    let centres = [*SOMA, eo_data::groundscale::plan([-200.0, 600.0]), eo_data::groundscale::plan([2600.0, 900.0]), eo_data::groundscale::plan([600.0, -1500.0])];
    let mut a = world_around(&centres, 0);
    let mut b = world_around(&centres, 3);
    let mut c = world_around(&centres, 0);
    run(&mut a, &centres, 1800);
    run(&mut b, &centres, 1800);
    run(&mut c, &centres, 1800);
    let (fa, fb, fc) = (fingerprint(&a), fingerprint(&b), fingerprint(&c));
    assert!(!fa.is_empty());
    assert_eq!(fa, fc, "two runs of one seed differ");
    assert_eq!(fa, fb, "one thread and four threads differ");
}

#[test]
fn tiles_stream_with_the_focus_and_agents_follow() {
    let src = source();
    let start = *SOMA;
    let mut world = World::new(Network::new(src.timing.clone()));
    let mut net_seed = Network::new(src.timing.clone());
    net_seed.add(src.build(&native::tiles_around(start[0], start[1], 400.0)));
    world.net = net_seed;
    world.regions.push(Region::new(&world.net, 0, RegionConfig::game_default(1), Focus { x: start[0], z: start[1], yaw: 0.0 }));
    // Walk the focus 2.4 km west at 15 m/s (a fast car), streaming tiles within 400 m.
    let (mut added, mut removed) = (0, 0);
    let mut peak_tiles = 0;
    for s in 0..(160 * 60) {
        let x = start[0] - 15.0 * s as f64 / 60.0;
        world.set_focus(0, Focus { x, z: start[1], yaw: -std::f64::consts::FRAC_PI_2 });
        if s % 30 == 0 {
            let (a, d) = native::stream(&mut world, src, 400.0);
            added += a;
            removed += d;
        }
        world.step();
        peak_tiles = peak_tiles.max(world.net.tiles.len());
    }
    let r = &world.regions[0];
    let (lanes, footways, nodes, foot_nodes, junctions, tiles) = world.net.resident();
    eprintln!("streamed: +{added} -{removed} tiles; resident {tiles} tiles, {lanes} lanes, {footways} footways, {nodes} nodes, {foot_nodes} footway nodes, {junctions} junctions; peak {peak_tiles} tiles");
    assert!(added > 20 && removed > 10, "tiles came and went (+{added} -{removed})");
    assert!(peak_tiles <= 64, "the resident set stays bounded ({peak_tiles} tiles)");
    assert!(world.net.lanes.len() < 6 * lanes.max(1), "lane slots are reused, not leaked");
    for (i, c) in r.fleet.cars.iter().enumerate() {
        assert!(world.net.lanes[c.lane as usize].alive, "car {i} on a dead lane");
    }
    for w in &r.crowd.walkers {
        assert!(world.net.footways[w.footway as usize].alive, "walker on a dead footway");
    }
    let f = r.focus;
    let near = (0..r.fleet.cars.len()).filter(|&i| (r.fleet.x[i] - f.x).hypot(r.fleet.z[i] - f.z) <= 240.0).count();
    assert!(near >= 10, "the fleet followed the focus ({near} within 240 m)");
    assert!(r.crowd.walkers.len() >= 20, "the disc refilled where the focus went ({})", r.crowd.walkers.len());
}

#[test]
fn native_signal_plans_obey_ca_mutcd() {
    let src = source();
    let mut net = Network::new(src.timing.clone());
    net.add(src.build(&native::tiles_around(SOMA[0], SOMA[1], 1200.0)));
    let t = &net.timing;
    let mut checked = 0;
    for j in net.junctions.iter().filter(|j| !j.orphan) {
        for a in j.axes.iter().filter(|a| !a.suppressed) {
            assert!(a.yellow_s + 1e-9 >= a.yellow_table_s && a.yellow_s >= t.yellow_min_s && a.yellow_s <= t.yellow_max_s);
            assert!(a.all_red_s >= t.all_red_min_s && a.all_red_s <= t.all_red_max_s);
            if let Some(p) = &a.ped {
                assert!(p.walk_s + 1e-9 >= t.ped_walk_s && p.span_s + 1e-9 >= p.required_total_s);
            }
            checked += 1;
        }
    }
    eprintln!("{} native junctions, {checked} phases checked", net.junctions.len());
    assert!(checked > 50);
}

/// Every byte of every agent record of every region.
fn records(world: &World) -> Vec<u8> {
    world.regions.iter().flat_map(|r| bytemuck::cast_slice::<AgentInstance, u8>(&r.out.vehicles).iter().chain(bytemuck::cast_slice::<AgentInstance, u8>(&r.out.walkers)).copied().collect::<Vec<u8>>()).collect()
}

#[test]
fn a_level_table_of_ways_at_grade_changes_nothing() {
    // (stage A5: lanes take their structure level from a table of way ids; a
    // table that leaves every way at grade, empty or listing them all at 0,
    // is the network and the world without one, byte for byte, every step.)
    let paths = eo_data::DataPaths::discover().unwrap();
    let every: std::collections::HashMap<String, i8> = eo_data::sim_source::SimSource::load(&paths).unwrap().roads.into_iter().map(|r| (r.id, 0)).collect();
    let listed = native::load(&paths).unwrap().with_levels(every);
    let empty = native::load(&paths).unwrap().with_levels(Default::default());
    assert!(source().levels().is_empty() && empty.levels().is_empty() && listed.levels().len() > 1000);
    let make = |src: &NativeSource| {
        let mut net = Network::new(src.timing.clone());
        net.add(src.build(&native::tiles_around(SOMA[0], SOMA[1], 600.0)));
        assert!(net.lanes.iter().all(|l| l.level == 0));
        let mut world = World::new(net);
        world.regions.push(Region::new(&world.net, 0, RegionConfig::game_default(1), focus_at(*SOMA, 0.0)));
        world
    };
    let mut worlds = [make(source()), make(&empty), make(&listed)];
    for s in 0..1200 {
        for w in worlds.iter_mut() {
            w.set_focus(0, focus_at(*SOMA, s as f64 / 60.0));
            w.step();
        }
        if s % 60 == 0 {
            let a = records(&worlds[0]);
            assert!(!a.is_empty());
            assert_eq!(a, records(&worlds[1]), "step {s}: an empty level table changed the world");
            assert_eq!(a, records(&worlds[2]), "step {s}: a table of ways at grade changed the world");
        }
    }
    assert_eq!(records(&worlds[0]), records(&worlds[2]));
}

#[test]
fn drawn_crossing_restores_a_node_between_simplified_ways() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficJunction, TrafficPlan, TrafficStop, TrafficStopControl};
    use eo_sim::geom::Movement;
    use eo_sim::network::ControlKind;

    let road = |id: &str, points| SimRoad {
        id: id.into(), name: id.into(), highway: "residential".into(), oneway: Oneway::Both,
        lanes: Some(2), maxspeed: String::new(), sidewalk: String::new(),
        bridge: false, tunnel: false, layer: 0, points,
    };
    let make = |first, second, signals| NativeSource::new(
        SimSource { roads: vec![road("east", first), road("south", second)], signals },
        source().dims.clone(), source().timing.clone(),
    );
    let east = || vec![[-60.0, 0.8], [60.0, 0.8]];
    let south = || vec![[0.55, -60.0], [0.55, 60.0]];
    let plan = || TrafficPlan {
        sections: vec![],
        junctions: vec![TrafficJunction {
            ways: vec!["east".into(), "south".into()],
            center: [0.0, 12.0, 0.0],
            triangles: vec![],
            rim: vec![[-8.0, 12.0, -8.0], [8.0, 12.0, -8.0], [8.0, 12.0, 8.0], [-8.0, 12.0, 8.0]],
        }],
        surveyed_routes: None,
    };
    let build = |src: NativeSource| {
        let mut net = Network::new(src.timing.clone());
        net.add(src.build(&native::tiles_around(0.0, 0.0, 90.0)));
        net
    };
    let raw = build(make(east(), south(), vec![]));
    assert_eq!(raw.alive_lanes().count(), 4);

    let net = build(make(east(), south(), vec![]).with_traffic_plan(plan()));
    assert_eq!(net.alive_lanes().count(), 8);
    let approach = |key: &str| net.alive_lanes().find(|&l| net.lanes[l as usize].key == key).unwrap();
    let eastbound = approach("neast:0-forward-1");
    let southbound = approach("nsouth:0-forward-1");
    let a = &net.lanes[eastbound as usize];
    let b = &net.lanes[southbound as usize];
    assert_eq!(a.end_node, b.end_node);
    assert_eq!(a.pts[0][1], a.pts.last().unwrap()[1], "same eastbound driving line");
    assert_eq!(b.pts[0][0], b.pts.last().unwrap()[0], "same southbound driving line");
    let through = |lane| net.lanes[lane as usize].links.iter().position(|l| {
        l.movement == Movement::Through && l.legal && l.drivable
    }).unwrap() as u16;
    let ix = [through(eastbound), through(southbound)];
    let node = a.end_node;
    let target = [a.links[ix[0] as usize].to, b.links[ix[1] as usize].to];
    let mut world = World::new(net);
    let mut cfg = RegionConfig::js_pool(2, 0);
    cfg.vehicle_pool = 2;
    let mut r = Region::new(&world.net, 0, cfg, Focus { x: 0.0, z: 0.0, yaw: 0.0 });
    for (i, lane) in [eastbound, southbound].into_iter().enumerate() {
        let length = world.net.lanes[lane as usize].length;
        r.fleet.place(&world.net, i, lane, length - 24.0, 8.0);
        r.fleet.cars[i].next = ix[i];
    }
    world.regions.push(r);
    let mut done = [false; 2];
    let mut claimed = [false; 2];
    for _ in 0..600 {
        world.step();
        let r = &world.regions[0];
        assert_eq!(eo_sim::stats::sample(&world.net, r, world.time).overlaps, 0);
        for i in 0..2 {
            done[i] |= r.fleet.cars[i].lane == target[i];
            claimed[i] |= r.fleet.cars[i].claim_node == node;
        }
    }
    assert!(done == [true; 2]);
    assert!(claimed == [true; 2]);

    let signal = build(make(east(), south(), vec![[0.0, 0.0]]).with_traffic_plan(plan()));
    assert_eq!(signal.junctions.len(), 1);
    let signed_stop = || TrafficStop {
        id: 2, position: [-12.0, 0.8],
        control: TrafficStopControl::Approach { way: "east".into(), toward: [1.0, 0.0], all_way: false },
    };
    let stop_net = |signal: bool, stops: Vec<TrafficStop>| {
        let signals = if signal { vec![[0.0, 0.0]] } else { vec![] };
        build(make(east(), south(), signals).with_traffic_plan(plan()).with_stops(stops))
    };
    let minor = stop_net(false, vec![signed_stop()]);
    let controls = |n: &Network| {
        n.alive_lanes().filter(|&l| matches!(n.lanes[l as usize].control, ControlKind::MappedStop { .. })).collect::<Vec<_>>()
    };
    let controlled = controls(&minor);
    assert_eq!(controlled.len(), 1);
    let lane = &minor.lanes[controlled[0] as usize];
    assert_eq!(lane.key, "neast:0-forward-1");
    assert!(matches!(lane.control, ControlKind::MappedStop { id: 2, all_way: false, .. }));
    let (x, ..) = eo_sim::geom::at_arc(&lane.pts, &lane.cum, lane.length - minor.stop_offset(controlled[0]));
    assert!((x + 12.0).abs() < 0.001);

    let all = stop_net(false, vec![TrafficStop {
        id: 1, position: [0.0, 0.0], control: TrafficStopControl::Junction { junction: vec!["east".into(), "south".into()] },
    }, signed_stop()]);
    let controlled = controls(&all);
    assert_eq!(controlled.len(), 4);
    assert!(controlled.iter().all(|&l| matches!(all.lanes[l as usize].control, ControlKind::MappedStop { all_way: true, .. })));
    assert!(controlled.iter().any(|&l| matches!(all.lanes[l as usize].control, ControlKind::MappedStop { id: 2, .. })));
    assert!(controls(&stop_net(true, vec![signed_stop()])).is_empty());

    let separated = build(make(east(), south(), vec![]).with_traffic_plan(plan())
        .with_levels([("south".into(), 1)].into()));
    assert_eq!(separated.alive_lanes().count(), 4);

    let mapped = build(make(
        vec![[-60.0, 0.8], [0.55, 0.8], [60.0, 0.8]],
        vec![[0.55, -60.0], [0.55, 0.8], [0.55, 60.0]],
        vec![],
    ).with_traffic_plan(plan()));
    assert_eq!(mapped.alive_lanes().count(), 8);
    assert!(mapped.nodes.iter().any(|n| n.pos == [0.55, 0.8]));

    let streaming = make(vec![[-800.0, 0.8], [800.0, 0.8]], south(), vec![])
        .with_traffic_plan(plan());
    let end_tile = eo_sim::TileKey::of(800.0, 0.8);
    let delivered = streaming.build(&[end_tile]);
    let mut tiles = native::tiles_around(0.0, 0.0, 90.0);
    tiles.push(end_tile);
    let together = streaming.build(&tiles);
    let isolated = &delivered.lanes[0];
    assert_eq!(isolated.key, "neast:1-forward-1");
    let combined = together.lanes.iter().find(|lane| lane.key == isolated.key).unwrap();
    assert_eq!(isolated.points, combined.points);
    assert_eq!(isolated.start_node, combined.start_node);
}

#[test]
fn a_collapsed_one_way_loop_cannot_create_lanes_or_sidewalks() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficLane, TrafficSection};

    let base = source();
    let road = |id: &str, oneway, points| SimRoad {
        id: id.into(), name: String::new(), highway: "residential".into(),
        oneway, lanes: Some(1), maxspeed: String::new(), sidewalk: "both".into(),
        bridge: false, tunnel: false, layer: 0, points,
    };
    let roads = vec![
        road("collapsed", Oneway::Forward, vec![[60.0, 60.0], [80.0, 60.0], [60.0, 60.0]]),
        road("reverse", Oneway::Reverse, vec![[60.0, 90.0], [80.0, 90.0], [60.0, 90.0]]),
        road("approach", Oneway::Forward, vec![[20.0, 60.0], [60.0, 60.0], [120.0, 60.0]]),
        road("loop", Oneway::Forward, vec![[120.0, 60.0], [150.0, 60.0], [150.0, 90.0], [120.0, 90.0], [120.0, 60.0]]),
        road("branch", Oneway::Forward, vec![[80.0, 60.0], [80.0, 90.0], [80.0, 110.0]]),
    ];
    let half = base.dims.city_lane_m / 2.0;
    let sections = roads.iter().flat_map(|road| {
        road.points.windows(2).map(move |points| TrafficSection {
            way: road.id.clone(), points: points.to_vec(), half,
            walks: [half; 2], walk_gaps: Default::default(), lanes: vec![TrafficLane {
                direction: if road.oneway == Oneway::Forward { 1 } else { -1 },
                lateral: 0.0, width: half * 2.0,
            }],
            shared_bidirectional: false, paint_span: None, paint: vec![],
            crosses_road: false, surface: vec![], carriageway_spans: vec![],
        })
    }).collect();
    let source = NativeSource::new(SimSource { roads, signals: vec![] }, base.dims.clone(), base.timing.clone());
    let tile = [eo_sim::TileKey::of(80.0, 60.0)];
    let raw = source.build(&tile);
    let paved = source.with_traffic_sections(sections).build(&tile);
    for build in [raw, paved] {
        let keys: Vec<_> = build.lanes.iter().map(|lane| lane.key.as_str()).collect();
        assert_eq!(keys, ["napproach:0-forward-1", "nloop:0-forward-1", "nbranch:0-forward-1"]);
        assert_eq!(build.lanes[0].end_node, build.lanes[1].start_node);
        assert!(build.footways.iter().any(|foot| foot.segment == "nloop:0"));
        assert!(build.footways.iter().all(|foot| matches!(foot.segment.as_str(), "napproach:0" | "nloop:0" | "nbranch:0")));
    }
}

#[test]
fn a_one_way_street_uses_the_drawn_chord_when_its_centerlines_diverge() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficLane, TrafficSection};

    let base = source();
    let mapped = [[20.0, 100.0], [120.0, 100.0]];
    let drawn = [[20.0, 101.0], [120.0, 99.0]];
    let road = SimRoad {
        id: "street".into(), name: "Street".into(), highway: "residential".into(),
        oneway: Oneway::Forward, lanes: Some(2), maxspeed: String::new(), sidewalk: "both".into(),
        bridge: false, tunnel: false, layer: 0, points: mapped.to_vec(),
    };
    let tile = [eo_sim::TileKey::of(70.0, 100.0)];
    let lane_width = base.dims.city_lane_m;
    let section = TrafficSection {
        way: "street".into(), points: drawn.into_iter().rev().collect(), half: lane_width / 2.0,
        walks: [base.dims.throughway_m; 2], walk_gaps: Default::default(), lanes: vec![TrafficLane {
            direction: -1, lateral: 0.0, width: lane_width,
        }],
        shared_bidirectional: false, paint_span: None, paint: vec![], crosses_road: false,
        surface: vec![], carriageway_spans: vec![],
    };
    let build = |section| NativeSource::new(
        SimSource { roads: vec![road.clone()], signals: vec![] }, base.dims.clone(), base.timing.clone(),
    ).with_traffic_sections(vec![section]).build(&tile);
    let paved = build(section.clone());
    assert_eq!(paved.lanes.len(), 1);
    let lane = &paved.lanes[0];
    let aligned = [drawn[0], [70.0, 100.0], drawn[1]];
    assert_eq!(lane.key, "nstreet:0-forward-1");
    assert_eq!(lane.points, aligned);
    assert_eq!([lane.start_node, lane.end_node], mapped);
    assert_eq!(paved.segments[0].points, aligned);
    assert_eq!(paved.footways.len(), 2);

    let mut opposing = section.clone();
    opposing.lanes[0].direction = 1;
    let fallback = build(opposing);
    assert_eq!(fallback.lanes.len(), 2);
    assert!(fallback.lanes.iter().all(|lane| [lane.start_node, lane.end_node] == mapped));

    let mut two_through_lanes = section;
    two_through_lanes.half = lane_width;
    two_through_lanes.lanes = [-lane_width / 2.0, lane_width / 2.0].into_iter()
        .map(|lateral| TrafficLane { direction: -1, lateral, width: lane_width }).collect();
    assert_eq!(build(two_through_lanes).lanes[0].points, fallback.lanes[0].points);
}

#[test]
fn a_registered_shared_lane_does_not_fall_back_to_two_autonomous_lanes() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficSection};

    let base = source();
    let source = NativeSource::new(SimSource { roads: vec![SimRoad {
        id: "street".into(), name: "Street".into(), highway: "residential".into(),
        oneway: Oneway::Both, lanes: Some(2), maxspeed: String::new(), sidewalk: "both".into(),
        bridge: false, tunnel: false, layer: 0, points: vec![[20.0, 100.0], [120.0, 100.0]],
    }], signals: vec![] }, base.dims.clone(), base.timing.clone());
    let tile = [eo_sim::TileKey::of(70.0, 100.0)];
    assert_eq!(source.build(&tile).lanes.len(), 2);

    // The two lines drift apart while staying within the same roadway.
    let half = base.dims.city_lane_m / 2.0;
    let section = TrafficSection {
        way: "street".into(), points: vec![[20.0, 100.0], [120.0, 100.6]], half,
        walks: [base.dims.throughway_m; 2], walk_gaps: Default::default(), lanes: vec![],
        shared_bidirectional: true, paint_span: None, paint: vec![], crosses_road: false,
        surface: vec![], carriageway_spans: vec![],
    };
    let matched = source.with_traffic_sections(vec![section]);
    let built = matched.build(&tile);
    assert!(built.lanes.is_empty());
    assert_eq!(built.footways.len(), 2);

    let deck = matched.with_levels([("street".into(), 1)].into()).build(&tile);
    assert_eq!(deck.lanes.len(), 2);
    assert!(deck.lanes.iter().all(|lane| lane.level == 1));
}

#[test]
fn an_approach_keeps_its_drawn_grade_over_lower_junction_asphalt() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficJunction, TrafficPlan, TrafficSection};

    let points = vec![[-40.0, 0.0], [0.0, 0.0]];
    let fixture = || SimSource { roads: vec![SimRoad {
        id: "rise".into(), name: "Approach".into(), highway: "residential".into(),
        oneway: Oneway::Forward, lanes: Some(1), maxspeed: String::new(), sidewalk: "no".into(),
        bridge: false, tunnel: false, layer: 0, points: points.clone(),
    }], signals: vec![] };
    let base = source();
    let section = |carriageway_spans| TrafficSection {
        way: "rise".into(), points: points.clone(), half: base.dims.city_lane_m / 2.0,
        walks: [0.0; 2], walk_gaps: Default::default(), lanes: vec![], shared_bidirectional: false,
        paint_span: None, paint: vec![], crosses_road: false,
        surface: vec![[-40.0, 13.0, 0.0], [-18.0, 13.0, 0.0], [-5.0, 10.0, 0.0], [0.0, 10.0, 0.0]],
        carriageway_spans,
    };
    let build = |spans| {
        let source = NativeSource::new(fixture(), base.dims.clone(), base.timing.clone())
            .with_traffic_plan(TrafficPlan {
                sections: vec![section(spans)],
                junctions: vec![TrafficJunction {
                    ways: vec!["rise".into()], center: [0.0, 10.0, 0.0],
                    triangles: vec![],
                    rim: vec![[-20.0, 10.0, -6.0], [8.0, 10.0, -6.0], [8.0, 10.0, 6.0], [-20.0, 10.0, 6.0]],
                }],
                surveyed_routes: None,
            });
        let mut net = Network::new(source.timing.clone());
        net.add(source.build(&native::tiles_around(0.0, 0.0, 60.0)));
        net
    };
    let legacy = build(vec![]);
    assert_eq!(legacy.lanes[0].surface_at(22.0), Some(10.0));
    let net = build(vec![[0, 2]]);
    let lane = &net.lanes[0];
    assert_eq!(lane.surface_at(22.0), Some(13.0));
    assert_eq!(lane.surface_at(35.0), Some(10.0));
    assert_eq!(lane.surface_at(37.0), Some(10.0));
    assert!((lane.surface_at(34.999).unwrap() - lane.surface_at(35.001).unwrap()).abs() < 0.001);
}

#[test]
fn a_trimmed_loop_keeps_its_paved_profile_through_the_shared_pad() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficJunction, TrafficLane, TrafficPlan, TrafficSection};

    let base = source();
    let points = vec![[40.0, 40.0], [80.0, 40.0], [80.0, 80.0], [40.0, 80.0], [40.0, 40.0]];
    let lane_width = base.dims.city_lane_m;
    let source = NativeSource::new(SimSource { roads: vec![SimRoad {
        id: "loop".into(), name: String::new(), highway: "service".into(),
        oneway: Oneway::Forward, lanes: Some(1), maxspeed: String::new(), sidewalk: "no".into(),
        bridge: false, tunnel: false, layer: 0, points: points.clone(),
    }], signals: vec![] }, base.dims.clone(), base.timing.clone()).with_traffic_plan(TrafficPlan {
        sections: vec![TrafficSection {
            way: "loop".into(), points, half: lane_width / 2.0, walks: [0.0; 2],
            walk_gaps: Default::default(), lanes: vec![TrafficLane {
                direction: 1, lateral: 0.0, width: lane_width,
            }],
            shared_bidirectional: false, paint_span: None, paint: vec![], crosses_road: false,
            surface: vec![
                [50.0, 12.0, 40.0], [80.0, 15.0, 40.0], [80.0, 16.0, 80.0],
                [40.0, 14.0, 80.0], [40.0, 12.0, 50.0],
            ],
            carriageway_spans: vec![[0, 4]],
        }],
        junctions: vec![TrafficJunction {
            center: [40.0, 12.0, 40.0], ways: vec!["loop".into()], triangles: vec![],
            rim: vec![[36.0, 12.0, 36.0], [50.0, 12.0, 36.0], [50.0, 12.0, 50.0], [36.0, 12.0, 50.0]],
        }],
        surveyed_routes: None,
    });
    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&[eo_sim::TileKey::of(40.0, 40.0)]));
    let lane = &net.lanes[net.lane_by_key("nloop:0-forward-1").unwrap() as usize];
    assert_eq!(lane.surface_at(0.0), Some(12.0));
    assert_eq!(lane.surface_at(lane.length / 2.0), Some(16.0));
    assert_eq!(lane.surface_at(lane.length), Some(12.0));
}

#[test]
fn the_drawn_mouth_completes_a_decks_traffic_surface() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficDeckSurface, TrafficJunction, TrafficPlan, TrafficSection};

    let road = |id: &str, points, bridge| SimRoad {
        id: id.into(), name: id.into(), highway: "residential".into(), oneway: Oneway::Both,
        lanes: Some(2), maxspeed: String::new(), sidewalk: String::new(),
        bridge, tunnel: false, layer: i32::from(bridge), points,
    };
    let junction = |y, ways| TrafficJunction {
        ways, center: [0.0, y, 0.0],
        triangles: vec![],
        rim: vec![[-10.0, y, -8.0], [10.0, y, -8.0], [10.0, y, 8.0], [-10.0, y, 8.0]],
    };
    let source = NativeSource::new(SimSource {
        roads: vec![
            road("deck", vec![[-40.0, 0.0], [0.0, 0.0]], true),
            road("street", vec![[0.0, 0.0], [40.0, 0.0]], false),
            road("lower", vec![[-5.0, -40.0], [-5.0, 40.0]], false),
        ],
        signals: vec![],
    }, source().dims.clone(), source().timing.clone())
        .with_levels([("deck".into(), 1)].into())
        .with_traffic_plan(TrafficPlan {
            sections: vec![TrafficSection {
                way: "street".into(), points: vec![[0.0, 0.0], [40.0, 0.0]], half: 4.0,
                walks: [0.0; 2], walk_gaps: [vec![], vec![]], lanes: vec![], shared_bidirectional: false, paint_span: None,
                paint: vec![], crosses_road: false, surface: vec![[0.0, 10.0, 0.0], [40.0, 10.0, 0.0]], carriageway_spans: vec![],
            }],
            junctions: vec![junction(2.0, vec!["deck".into()]), junction(10.0, vec!["deck".into(), "street".into()])],
            surveyed_routes: None,
        }).with_deck_surfaces(vec![TrafficDeckSurface {
            // The exact deck mouth rounds to the f32 junction rim at x=-10.
            way: "deck".into(), points: vec![[-40.0, 11.0, 0.0], [-10.0000004, 10.0, 0.0]],
            lanes: vec![],
        }]);
    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&native::tiles_around(0.0, 0.0, 60.0)));
    let deck = &net.lanes[net.lane_by_key("ndeck:0-forward-1").unwrap() as usize];
    assert_eq!(deck.surface_at(0.0), Some(11.0));
    assert_eq!(deck.surface_at(deck.length), Some(10.0));
    assert!(
        (deck.surface_at(29.999).unwrap() - deck.surface_at(30.001).unwrap()).abs() < 0.001,
        "the approach height stays continuous at the mouth",
    );
    assert!(deck.links.iter().any(|l| l.legal && l.drivable
        && net.lanes[l.to as usize].key == "nstreet:0-forward-1"));
    assert!(!deck.links.iter().any(|l| net.lanes[l.to as usize].key.starts_with("nlower:")));
}

#[test]
fn painted_deck_tracks_supply_the_position_width_and_grade() {
    use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficDeckLane, TrafficDeckSurface};

    let road = SimRoad {
        id: "bridge".into(), name: "Bridge".into(), highway: "motorway".into(),
        oneway: Oneway::Reverse, lanes: Some(1), maxspeed: String::new(),
        sidewalk: String::new(), bridge: true, tunnel: false, layer: 1,
        points: vec![[0.0, 0.0], [40.0, 0.0]],
    };
    let source = NativeSource::new(SimSource { roads: vec![road], signals: vec![] },
        source().dims.clone(), source().timing.clone())
        .with_levels([("bridge".into(), 1)].into())
        .with_deck_surfaces(vec![TrafficDeckSurface {
            way: "bridge".into(), points: vec![[0.0, 9.0, 0.0], [40.0, 9.0, 0.0]],
            lanes: vec![TrafficDeckLane {
                width: 3.0,
                points: vec![[0.0, 10.0, 2.0], [20.0, 11.0, 5.0], [40.0, 12.0, 2.0]],
            }],
        }]);
    let build = source.build(&native::tiles_around(0.0, 0.0, 60.0));
    let lane = &build.lanes[0];
    assert_eq!(lane.key, "nbridge:0-reverse-1");
    assert_eq!(lane.points, [[40.0, 2.0], [20.0, 5.0], [0.0, 2.0]]);
    assert_eq!(lane.width, 3.0);
    assert_eq!([lane.start_node, lane.end_node], [[40.0, 0.0], [0.0, 0.0]]);
    let heights: Vec<_> = build.lane_surfaces[0].1.iter().map(|sample| sample[1]).collect();
    assert_eq!(heights, [12.0, 11.0, 10.0]);
}

/// The corridor layer keeps every posted second, however the posts are
/// batched (the regions drop what `MAX_STEPS_PER_ADVANCE` cannot catch up):
/// two worlds fed the same 2 s, one a frame at a time and one in a lump,
/// draw the same corridor traffic.
#[test]
fn the_corridor_layer_replays_however_the_time_is_posted() {
    use eo_sim::corridor::{Corridors, Params, Travel, WaySpec};
    let way = WaySpec {
        name: "test".into(),
        points: vec![[0.0, 10.0, 0.0], [0.0, 10.0, -3000.0]],
        lanes: vec![1.8, 5.4],
        speed_mps: 20.0,
        flow_vph: [2500.0; 24],
        mix: [0.9, 0.02, 0.06, 0.02],
        travel: Travel::Vehicles,
        hidden: vec![],
        handover: false,
        next: None,
    };
    let world = || {
        let mut w = World::new(Network::new(source().timing.clone()));
        w.corridors = Some(Corridors::new(vec![way.clone()], Params::with_time_gap(3.0), 4));
        w
    };
    let (mut a, mut b) = (world(), world());
    for _ in 0..120 {
        a.advance(1.0 / 60.0);
    }
    b.advance(1.5);
    b.advance(0.5);
    assert_eq!(a.corridor_steps, 120);
    assert_eq!(b.corridor_steps, 120);
    let (ca, cb) = (a.corridors.as_ref().unwrap(), b.corridors.as_ref().unwrap());
    assert!(!ca.out.is_empty());
    assert_eq!(ca.out, cb.out);
}

/// (`--car-density`, `TrafficDensity`) The fleet put down by street class and
/// hour: at the busy hour SoMa's streets carry half as many again as the JS
/// pool's cars round the focus, most of them on its arterials, none on top of
/// another; at four in the morning the JS pool's count; off (None) the JS pool as before.
#[test]
fn the_traffic_density_follows_the_streets_and_the_hour() {
    use eo_sim::consts::{DENSE_VEHICLE_POOL, VEHICLE_POOL};
    use eo_sim::region::TrafficDensity;
    let src = source();
    let mut net = Network::new(src.timing.clone());
    net.add(src.build(&native::tiles_around(SOMA[0], SOMA[1], 600.0)));
    let focus = Focus { x: SOMA[0], z: SOMA[1], yaw: 0.0 };
    let region = |traffic: Option<f64>| {
        let mut cfg = RegionConfig::game_default(1);
        if let Some(h) = traffic {
            cfg.vehicle_pool = DENSE_VEHICLE_POOL;
            cfg.traffic = Some(TrafficDensity::standard(h));
        }
        Region::new(&net, 0, cfg, focus)
    };
    let near = |r: &Region| (0..r.fleet.len()).filter(|&i| ((r.fleet.x[i] - focus.x).powi(2) + (r.fleet.z[i] - focus.z).powi(2)).sqrt() < 120.0).count();
    let off = region(None);
    assert_eq!(off.fleet.len(), VEHICLE_POOL, "off: the JS pool");
    let busy = region(Some(17.0));
    let small = region(Some(4.0));
    // the cars the streets carry within the radius at their classes' densities (SoMa's two collectors: ~65)
    let t = TrafficDensity::standard(17.0);
    let expected: f64 = net.lanes.iter().filter(|l| l.alive).map(|l| t.lane_cars(l, [focus.x, focus.z])).sum();
    assert!(expected > 1.3 * VEHICLE_POOL as f64, "SoMa at 5 pm: {expected:.0} cars expected");
    assert!(busy.fleet.len() as f64 >= 0.9 * expected.min(DENSE_VEHICLE_POOL as f64) && busy.fleet.len() <= DENSE_VEHICLE_POOL, "busy: {} of {expected:.0}", busy.fleet.len());
    // a quiet hour keeps the JS pool's cars and adds only where an arterial is left short
    assert!(small.fleet.len() + 3 >= VEHICLE_POOL && small.fleet.len() < busy.fleet.len(), "4 am {} vs 5 pm {}", small.fleet.len(), busy.fleet.len());
    // the JS pool's cars are put down where they were without the density (the same lanes and draws), the rest after them
    let kept = (0..off.fleet.len().min(busy.fleet.len()))
        .filter(|&i| busy.fleet.cars[i].lane == off.fleet.cars[i].lane && (busy.fleet.cars[i].arc - off.fleet.cars[i].arc).abs() < 8.0)
        .count();
    assert!(kept * 10 >= off.fleet.len() * 9, "{kept} of the pool's {} cars where they were", off.fleet.len());
    assert!(near(&busy) > near(&off), "near the focus: {} vs {}", near(&busy), near(&off));
    // the arterials carry most of them
    let class = |r: &Region, i: usize| net.lanes[r.fleet.cars[i].lane as usize].highway.clone();
    let arterial = (0..busy.fleet.len()).filter(|&i| matches!(class(&busy, i).as_str(), "primary" | "secondary" | "primary_link" | "secondary_link")).count();
    assert!(arterial * 2 > busy.fleet.len(), "{arterial} of {} on primary or secondary streets", busy.fleet.len());
    // no two on one lane closer than a car's length
    for i in 0..busy.fleet.len() {
        for j in i + 1..busy.fleet.len() {
            let (a, b) = (&busy.fleet.cars[i], &busy.fleet.cars[j]);
            if a.lane == b.lane {
                assert!((a.arc - b.arc).abs() >= 4.0, "cars {i} and {j} overlap on lane {}", a.lane);
            }
        }
    }
    // the falloff: whole near, a quarter at the radius, none past it
    let t = TrafficDensity::standard(12.0);
    assert_eq!(t.falloff(0.0), 1.0);
    assert!((t.falloff(t.radius_m) - 0.25).abs() < 1e-9 && t.falloff(t.radius_m + 1.0) == 0.0);
}

#[test]
fn surveyed_circuit_tiles_arrive_and_leave_as_a_group() {
    use eo_data::sim_source::{
        Oneway, SimRoad, SimSource, SurveyedTrafficApproach, SurveyedTrafficLine,
        SurveyedTrafficRoutes, TrafficPlan,
    };
    use eo_sim::network::TileKey;

    let baseline = source();
    let road = |id: &str, points| SimRoad {
        id: id.into(), name: id.into(), highway: "residential".into(), oneway: Oneway::Forward,
        lanes: Some(1), maxspeed: String::new(), sidewalk: String::new(), bridge: false,
        tunnel: false, layer: 0, points,
    };
    let make = || NativeSource::new(SimSource {
        roads: vec![
            road("entry", vec![[240.0, 60.0], [240.0, 95.0]]),
            road("exit", vec![[228.0, 110.0], [-30.0, 110.0]]),
            road("collapsed", vec![[240.0, 100.0], [220.0, 100.0], [240.0, 100.0]]),
        ],
        signals: vec![],
    }, baseline.dims.clone(), baseline.timing.clone());
    let line = |points: Vec<_>| SurveyedTrafficLine { heights: vec![2.0; points.len()], points };
    let mut boundary: Vec<_> = (0..128).map(|i| {
        let angle = std::f64::consts::TAU * f64::from(i) / 128.0;
        [230.0 + 10.0 * angle.cos(), 100.0 + 10.0 * angle.sin()]
    }).collect();
    boundary.push(boundary[0]);
    let plan = TrafficPlan { surveyed_routes: Some(SurveyedTrafficRoutes {
        collapsed_way: "collapsed".into(), vehicle_envelope_m: [6.0, 2.6],
        approaches: vec![
            SurveyedTrafficApproach { lane: "nentry:0-forward-1".into(), tile: [0, 0],
                inbound: true, stop_id: None, line: line(vec![[240.0, 60.0], boundary[0]]) },
            SurveyedTrafficApproach { lane: "nexit:0-forward-1".into(), tile: [-1, 0],
                inbound: false, stop_id: None, line: line(vec![boundary[32], [-30.0, 110.0]]) },
        ],
        circulation: (0..4).map(|i| line(boundary[i*32..=(i+1)*32].to_vec())).collect(),
    }), ..Default::default() };

    let inlet = TileKey { x: 0, z: 0 };
    let outlet = TileKey { x: -1, z: 0 };
    let no_models = make().with_traffic_plan(plan.clone());
    assert_eq!(no_models.delivery_tiles(inlet), vec![inlet]);
    let report = std::sync::Arc::new(eo_sim::vehicle_catalogue::ScaleReport::read(
        br#"{"avatar_height_m":1.7,"vehicles":[{"model":"small","chassis_contact":{"min_xz_m":[-1,-2],"max_xz_m":[1,2]}}],"simulation_footprints":[{"kind":"sedan","catalogue_models":["small"]},{"kind":"taxi","catalogue_models":["small"]},{"kind":"truck","catalogue_models":["small"]},{"kind":"bus","catalogue_models":["small"]}]}"#.as_slice(),
    ).unwrap());
    let source = make().with_traffic_plan(plan).with_model_footprints(report);
    assert_eq!(source.delivery_tiles(inlet), vec![outlet, inlet]);
    assert_eq!(source.delivery_tiles(outlet), vec![outlet, inlet]);

    let mut world = World::new(source.new_network());
    let mut cfg = RegionConfig::game_default(7);
    cfg.vehicle_pool = 0;
    cfg.people = eo_sim::People::None;
    world.regions.push(Region::new(&world.net, 0, cfg, Focus { x: 240.0, z: 100.0, yaw: 0.0 }));
    let connected = |net: &Network| {
        let approach = net.lane_by_key("nentry:0-forward-1").unwrap();
        let first = net.lane_by_key("surveyed-collapsed:0").unwrap();
        let exit = net.lane_by_key("nexit:0-forward-1").unwrap();
        assert!(net.lanes[approach as usize].links.iter().any(|l| l.to == first && l.drivable));
        assert!(net.lanes[first as usize].links.iter().any(|l| l.to == exit && l.drivable));
        assert_eq!(net.lanes[first as usize].surface_at(0.0), Some(2.0));
        assert!(!net.lanes[approach as usize].fits_vehicle(10.0, 2.3));
    };
    assert_eq!(native::stream(&mut world, &source, 0.0).0, 2);
    connected(&world.net);
    world.set_focus(0, Focus { x: 850.0, z: 100.0, yaw: 0.0 });
    assert_eq!(native::stream(&mut world, &source, 0.0).1, 2);
    assert!(world.net.lane_by_key("surveyed-collapsed:0").is_none());
    world.set_focus(0, Focus { x: -30.0, z: 100.0, yaw: 0.0 });
    assert_eq!(native::stream(&mut world, &source, 0.0).0, 2);
    connected(&world.net);
}
