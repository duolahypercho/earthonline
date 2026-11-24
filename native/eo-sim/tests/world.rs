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
    // a quiet hour keeps the JS pool's cars (on the busier streets)
    assert!(small.fleet.len() <= VEHICLE_POOL && small.fleet.len() + 3 >= VEHICLE_POOL && small.fleet.len() < busy.fleet.len(), "4 am {} vs 5 pm {}", small.fleet.len(), busy.fleet.len());
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
