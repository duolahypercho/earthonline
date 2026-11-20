//! Cars in the tunnels' bores (`--structures --tunnels`), on the real city:
//! the feed marks a car on a lane under grade inside a bore with
//! `AGENT_IN_BORE` and its depth in from the nearer portal (the renderer lights
//! it by the bore, eo-render agents_draw.wgsl), and marks nothing else: a car
//! at grade over the bore, a car on the same lane past the portal, a car
//! without the tunnels.

use std::sync::Arc;

use eo_agents::feed::{Feeder, Ground, in_bore};
use eo_agents::library::Library;
use eo_agents::{AGENT_IN_BORE, BORE_DEPTH_QUANT, GpuModel, bore_depth_of_flags, bore_flags};
use eo_sim::output::{AgentBuffer, AgentInstance, level_flags};
use eo_sim::world::Frame;
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::{StructureOptions, StructureWorld};

fn library() -> Arc<Library> {
    let mut lib = Library::default();
    let mut m = GpuModel::default();
    m.wheel = [[0.8, 0.33, 1.4, 0.0], [-0.8, 0.33, 1.4, 0.0], [0.8, 0.33, -1.4, 0.0], [-0.8, 0.33, -1.4, 0.0]];
    m.wheel_radius = 0.33;
    lib.models.push(m);
    lib.meta.model_names.push("sedan".into());
    lib.meta.vehicle_models.insert("sedan".into(), 0);
    Arc::new(lib)
}

#[test]
fn the_bore_flags_round_trip_and_saturate() {
    for d in [0.0, 0.06, 12.3, 30.0, 81.0, 640.25] {
        let f = bore_flags(d);
        assert!(f & AGENT_IN_BORE != 0);
        let back = bore_depth_of_flags(f).unwrap();
        assert!((back - d).abs() <= 0.5 / BORE_DEPTH_QUANT + 1e-9, "{d} -> {back}");
        // the low bits (the simulation's and the lamps' flags, the player bit) stay free.
        assert_eq!(f & 0x7_ffff, AGENT_IN_BORE);
    }
    let deep = bore_depth_of_flags(bore_flags(f64::INFINITY)).unwrap();
    assert!(deep > 1000.0, "no portal reads as deep inside: {deep}");
    assert!(bore_depth_of_flags(bore_flags(1e7)).unwrap() == deep);
    assert_eq!(bore_depth_of_flags(0x2_0000), None, "the blink bit is not the bore's");
}

#[test]
fn a_car_in_a_bore_is_marked_with_its_depth_and_nothing_else_is() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let o = StreetOptions::load().unwrap();
    let sc = StreetCity::load().unwrap();
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let streets = Arc::new(StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings)));
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let lm = eo_landmarks::LandmarksJs::get();
    let tunnels = Arc::new(StructureWorld::build(&streets, &city, &ca, lm, StructureOptions { tunnels: true }).unwrap());
    let plain = Arc::new(StructureWorld::build(&streets, &city, &ca, lm, StructureOptions::default()).unwrap());
    assert!(plain.bores().is_empty(), "no bores without the tunnels");
    // the longest bore with a portal, and a station 40 m in from it.
    let bore = tunnels.bores().iter().filter(|b| b.portal[0] && b.s1 - b.s0 > 120.0).max_by(|a, b| (a.s1 - a.s0).total_cmp(&(b.s1 - b.s0))).expect("a tunnel bore with a portal");
    let level = tunnels.chain(bore.chain).map(|c| c.level).unwrap_or(-1);
    assert!(level < 0, "a bore's chain is under grade: level {level}");
    let at = |s: f64| {
        let w = bore.points.windows(2).find(|w| w[0].s <= s && w[1].s >= s).unwrap();
        let t = (s - w[0].s) / (w[1].s - w[0].s).max(1e-9);
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        ([lerp(w[0].x, w[1].x), lerp(w[0].z, w[1].z)], lerp(w[0].y, w[1].y), [w[1].x - w[0].x, w[1].z - w[0].z])
    };
    let (p, y, dir) = at(bore.s0 + 40.0);
    let hit = tunnels.bore_at(p[0], y + 1.0, p[1]).expect("the station is inside the bore");
    println!("bore of chain {} ({:.0}-{:.0} m, portals {:?}); car 40 m in at ({:.1}, {:.1}, {:.1}): depth {:.2} m", bore.chain, bore.s0, bore.s1, bore.portal, p[0], y, p[1], hit.depth);
    assert!((hit.depth - 40.0).abs() < 1.0, "depth {}", hit.depth);
    assert_eq!(in_bore(Some(&tunnels), level, p[0], y, p[1]), Some(hit.depth));
    // at grade (the street over the hill), on a deck, without structures, without the tunnels: none.
    assert_eq!(in_bore(Some(&tunnels), 0, p[0], y, p[1]), None);
    assert_eq!(in_bore(Some(&tunnels), 1, p[0], y, p[1]), None);
    assert_eq!(in_bore(None, level, p[0], y, p[1]), None);
    assert_eq!(in_bore(Some(&plain), level, p[0], y, p[1]), None);
    // under grade but out in the open cut, 20 m before the portal: none.
    let out = bore.points[0];
    let back = (dir[0].hypot(dir[1])).max(1e-9);
    let (ox, oz) = (out.x - dir[0] / back * 20.0, out.z - dir[1] / back * 20.0);
    assert_eq!(in_bore(Some(&tunnels), level, ox, out.y, oz), None, "the open cut before the portal");

    // the feed, end to end: the car in the bore, a car at grade on the hill over it.
    let ground = Ground::Street(streets.clone());
    let origin = [p[0].div_euclid(256.0) * 256.0, p[1].div_euclid(256.0) * 256.0];
    let heading = dir[0].atan2(dir[1]) as f32;
    let car = |id: u32, level: i8, flags: u16| AgentInstance { pos: [(p[0] - origin[0]) as f32, 0.0, (p[1] - origin[1]) as f32], heading, speed: 11.0, anim: 1.0, kind: 0, flags: level_flags(level) | flags, id };
    let frame = Frame { time: 1.0, steps: 60, regions: vec![AgentBuffer { origin, time: 1.0, step: 60, vehicles: vec![car(1, level, 1), car(2, 0, 1)], walkers: vec![] }], timing: Default::default() };
    let mut feeder = Feeder::new(library(), ground.clone(), Some(tunnels.clone()), 7).unwrap();
    let mut fed = vec![];
    feeder.build(&frame, &mut fed);
    assert_eq!(fed.len(), 2);
    assert_eq!(feeder.in_bores, 1);
    let d = bore_depth_of_flags(fed[0].flags).expect("the car in the bore is marked");
    println!("fed: the car in the bore at y {:.2} carries depth {d:.3} (flags {:#x}); the car on the hill flags {:#x}", fed[0].pos[1], fed[0].flags, fed[1].flags);
    assert!((d - hit.depth).abs() <= 1.0, "fed depth {d} vs {}", hit.depth);
    assert_eq!(fed[0].flags & 0xffff, 1, "the simulation's flags kept (braking), the level taken off");
    assert_eq!(fed[1].flags & AGENT_IN_BORE, 0, "a car at grade over the bore is not in it");
    // without the tunnels, the same frame: nothing is marked and the flags are as before.
    let mut feeder = Feeder::new(library(), ground, Some(plain), 7).unwrap();
    let mut before = vec![];
    feeder.build(&frame, &mut before);
    assert_eq!(feeder.in_bores, 0);
    assert!(before.iter().all(|a| a.flags & !0xffff == 0), "no bore bits without bores");
}
