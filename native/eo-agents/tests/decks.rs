//! Cars on the grade-separated roads (`--structures`), on the real city: a
//! car on a structure level's lane stands on that level's deck profile,
//! pitched to its grade, while a car at grade under the deck stays on the
//! street; the simulation, given the structures' level table, runs traffic
//! over the freeway decks, and the feed sets every car on a deck within a
//! few centimetres of the deck as drawn.

use std::sync::{Arc, OnceLock};

use eo_agents::feed::{Feeder, Ground, car_ground, car_ground_on_route};
use eo_agents::library::Library;
use eo_agents::{GpuAgent, GpuModel};
use eo_sim::native::{self, NativeSource};
use eo_sim::output::{AgentBuffer, AgentInstance, level_flags, level_of_flags};
use eo_sim::region::{Focus, Region, RegionConfig};
use eo_sim::world::Frame;
use eo_sim::{Network, World};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::{ChainKind, RoadClass, StructureClass, StructureOptions, StructureWorld};

struct City {
    streets: Arc<StreetWorld>,
    structures: Arc<StructureWorld>,
}

fn city() -> &'static City {
    static C: OnceLock<City> = OnceLock::new();
    C.get_or_init(|| {
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
        let structures = Arc::new(StructureWorld::build(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions::default()).unwrap());
        City { streets, structures }
    })
}

/// A library with one body, a sedan of wheelbase 2.8 m (the feed's pick
/// falls back to it whatever body it draws).
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

/// The pitch of a GPU instance (rotation about x after the heading).
fn pitch_of(a: &GpuAgent) -> f32 {
    let q = glam::Quat::from_array(a.rot);
    let (_, x, _) = q.to_euler(glam::EulerRot::YXZ);
    -x
}

/// The drawn deck surfaces (near render meshes) over (x, z): their heights.
fn drawn_decks(w: &StructureWorld, x: f64, z: f64) -> Vec<f64> {
    let key = eo_street::tile_key(x, z);
    let mut out = vec![];
    for dz in -1..=1 {
        for dx in -1..=1 {
            let Some(m) = w.render_tile_shared((key.0 + dx, key.1 + dz), 0) else { continue };
            for t in m.indices.chunks_exact(3) {
                if m.classes[t[0] as usize] != StructureClass::DeckSurface as u8 {
                    continue;
                }
                let p = |i: u32| m.positions[i as usize].map(|v| v as f64);
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                if det.abs() < 1e-12 {
                    continue;
                }
                let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
                let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9 {
                    out.push(l1 * a[1] + l2 * b[1] + l3 * c[1]);
                }
            }
        }
    }
    out
}

#[test]
fn a_car_on_a_deck_lane_stands_on_the_deck_and_a_car_under_it_on_the_street() {
    let c = city();
    let sw = &*c.structures;
    let ground = Ground::Street(c.streets.clone());
    // James Lick Freeway over Cortland Avenue (sf-seg-2069; stage A4's walker point).
    let (x, z) = (1239.0, 5301.7);
    let decks = sw.decks_at(x, z);
    let (level, deck) = *decks.iter().find(|(l, _)| *l >= 1).expect("a deck over the street");
    let street = ground.y(x, z);
    assert!(deck.y > street + 4.5, "deck {} over street {street}", deck.y);
    let heading = deck.dir;
    let (y, pitch) = car_ground(&ground, Some(sw), level, x, z, heading, 1.4);
    let want_pitch = deck.slope_toward(heading).atan() as f32;
    println!("level {level}: car at {y:.3}, deck {:.3}, street {street:.3}; pitch {pitch:.4} vs the deck's {want_pitch:.4}", deck.y);
    assert!((y - deck.y).abs() <= 0.05, "car {y} vs deck {}", deck.y);
    assert!((pitch - want_pitch).abs() <= 0.005, "pitch {pitch} vs {want_pitch}");
    // the same place at grade: on the street, not lifted onto the deck.
    let (y0, _) = car_ground(&ground, Some(sw), 0, x, z, heading, 1.4);
    let yb = 0.5 * (ground.y(x + heading[0] * 1.4, z + heading[1] * 1.4) + ground.y(x - heading[0] * 1.4, z - heading[1] * 1.4));
    assert_eq!(y0.to_bits(), yb.to_bits());
    assert!((y0 - street).abs() < 0.05);
    let (on_route, _) = car_ground_on_route(&ground, Some(sw), level, x, z, heading, 1.4, Some(deck.y));
    let (under_route, _) = car_ground_on_route(&ground, Some(sw), 0, x, z, heading, 1.4, Some(street));
    assert!((on_route - y).abs() < 0.05);
    assert!((under_route - y0).abs() < 0.05);
    // and without structures, whatever the level: the street.
    assert_eq!(car_ground(&ground, None, level, x, z, heading, 1.4).0.to_bits(), y0.to_bits());

    // the feed, end to end: one car of each level at the point, heading along the deck.
    let mut feeder = Feeder::new(library(), ground.clone(), Some(c.structures.clone()), 7).unwrap();
    let origin = [x.div_euclid(256.0) * 256.0, z.div_euclid(256.0) * 256.0];
    let car = |id: u32, level: i8| AgentInstance {
        pos: [(x - origin[0]) as f32, 0.0, (z - origin[1]) as f32],
        heading: (heading[0].atan2(heading[1])) as f32,
        speed: 20.0,
        anim: 0.0,
        kind: 0,
        flags: level_flags(level),
        id,
    };
    let frame = Frame { time: 1.0, steps: 60, regions: vec![AgentBuffer { origin, time: 1.0, step: 60, vehicles: vec![car(1, level), car(2, 0)], walkers: vec![] }], ..Default::default() };
    let mut out = vec![];
    feeder.build(&frame, &mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(feeder.on_decks, 1);
    let (up, down) = (&out[0], &out[1]);
    assert!((up.pos[1] as f64 - deck.y).abs() <= 0.05, "fed car on the deck at {} vs {}", up.pos[1], deck.y);
    assert!((down.pos[1] as f64 - street).abs() <= 0.05, "fed car under the deck at {} vs the street {street}", down.pos[1]);
    assert!((pitch_of(up) - want_pitch).abs() <= 0.005, "fed pitch {} vs {want_pitch}", pitch_of(up));
    // the level is the feed's; the renderer gets the lamp flags only.
    assert!(out.iter().all(|a| a.flags & 0x0F00 == 0));
}

#[test]
fn traffic_on_the_freeway_decks_stands_on_them() {
    let c = city();
    let sw = &*c.structures;
    let paths = eo_data::DataPaths::discover().unwrap();
    let table = sw.way_levels().clone();
    let source: NativeSource = native::load(&paths).unwrap().with_levels(table.clone());
    // the sim's ways and the structures' table name ways alike.
    let sim_ways: std::collections::HashSet<String> = eo_data::sim_source::SimSource::load(&paths).unwrap().roads.into_iter().map(|r| r.id).collect();
    let named = table.keys().filter(|k| sim_ways.contains(*k)).count();
    println!("level table: {} ways, {named} of them roads of the simulation", table.len());
    assert!(table.len() > 100 && named * 100 >= table.len() * 95, "{named} of {}", table.len());
    // a focus on the I-80 approach's longest built mainline deck.
    let main = sw
        .chains()
        .iter()
        .filter(|c| c.is_built() && c.kind == ChainKind::Deck && c.class == RoadClass::Motorway && c.pieces.iter().any(|p| p.name.contains("80")))
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .expect("an I-80 deck");
    let f = main.frame(main.length() / 2.0);
    let mut net = Network::new(source.timing.clone());
    net.add(source.build(&native::tiles_around(f.x, f.z, 700.0)));
    let deck_lanes = net.lanes.iter().filter(|l| l.alive && l.level != 0).count();
    assert!(deck_lanes > 20, "lanes on a structure level: {deck_lanes}");
    // no walker on a structure: no footway of a way on a structure level.
    let deck_segments: std::collections::HashSet<u32> = net.lanes.iter().filter(|l| l.alive && l.level != 0).map(|l| l.segment).collect();
    let footways = net.footways.iter().filter(|f| f.alive).count();
    assert!(footways > 100 && net.footways.iter().filter(|f| f.alive).all(|f| !deck_segments.contains(&f.segment)), "a footway on a structure level");
    let mut world = World::new(net);
    world.regions.push(Region::new(&world.net, 0, RegionConfig::game_default(3), Focus { x: f.x, z: f.z, yaw: f.tx.atan2(f.tz) }));
    let ground = Ground::Street(c.streets.clone());
    let blend = 12.0; // eo-structures' CROWN_BLEND_M: the deck top blends to a paved end's street over this.
    let (mut samples, mut fallback, mut near_end) = (0usize, 0usize, 0usize);
    let (mut wheel, mut centre, mut pitch_err, mut centre_all) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut ids: std::collections::BTreeSet<u32> = Default::default();
    let mut at_end = 0;
    let mut worst_at = (0u32, 0.0f64, 0.0f64, 0.0f64);
    for step in 0..3600 {
        world.step();
        if step % 30 != 29 {
            continue;
        }
        let r = &world.regions[0];
        let [ox, oz] = r.out.origin;
        at_end = 0;
        for a in &r.out.vehicles {
            let level = level_of_flags(a.flags);
            if level == 0 {
                continue;
            }
            at_end += 1;
            let (x, z) = (ox + a.pos[0] as f64, oz + a.pos[2] as f64);
            let h = [(a.heading as f64).sin(), (a.heading as f64).cos()];
            let (y, pitch) = car_ground(&ground, Some(sw), level, x, z, h, 1.4);
            let Some(d) = sw.deck_at(x, z, level) else {
                // (an end of a lane past its deck: on the street, as before.)
                fallback += 1;
                continue;
            };
            samples += 1;
            ids.insert(a.id);
            // each wheel axle on the deck as drawn (the render mesh's deck triangles under it).
            for k in [1.0, -1.0] {
                let (px, pz) = (x + h[0] * 1.4 * k, z + h[1] * 1.4 * k);
                let Some(top) = sw.deck_top(d.chain, px, pz) else { continue };
                if let Some(e) = drawn_decks(sw, px, pz).iter().map(|v| (v - top).abs()).min_by(|p, q| p.total_cmp(q)) {
                    wheel = wheel.max(e);
                }
            }
            // the car on its deck's profile, pitched to its grade (away from a paved end's blend).
            let ch = sw.chain(d.chain).unwrap();
            let paved = |e: usize| ch.ends[e].kind == eo_structures::EndKind::Paved;
            centre_all = centre_all.max((y - d.y).abs());
            if (paved(0) && d.s < blend) || (paved(1) && d.s > ch.length() - blend) {
                near_end += 1;
                continue;
            }
            centre = centre.max((y - d.y).abs());
            let pe = (pitch as f64 - d.slope_toward(h).atan()).abs();
            if pe > pitch_err {
                worst_at = (d.chain, d.s, x, z);
            }
            pitch_err = pitch_err.max(pe);
        }
    }
    println!(
        "60 s of traffic about ({:.0}, {:.0}) on chain {}: {} cars drove on structure lanes ({at_end} at the end of the minute), {samples} placements on a deck (every 0.5 s; {near_end} within {blend} m of a paved end), {fallback} past a deck's end; wheels within {wheel:.4} m of the drawn deck; away from paved ends the car within {centre:.4} m of its deck's profile (deck_at) and pitched within {pitch_err:.4} rad of its grade at the centre (the most at chain {} s {:.1}, where the profile turns under the car), within {centre_all:.3} m of the profile anywhere",
        f.x,
        f.z,
        main.id,
        ids.len(),
        worst_at.0,
        worst_at.1
    );
    assert!(ids.len() >= 3 && samples > 50, "{} cars, {samples} placements on decks", ids.len());
    println!("{deck_lanes} lanes on a structure level, on {} street segments; {footways} footways, none on a structure level", deck_segments.len());
    // (the wheels stand on the drawn deck's own triangles to within the mesh's linear steps.)
    assert!(wheel <= 0.03, "a wheel {wheel:.3} m off the drawn deck");
    assert!(centre <= 0.05 && pitch_err <= 0.05, "centre {centre:.3} m, pitch {pitch_err:.4}");
}
