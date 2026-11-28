use super::*;
use glam::DVec2;

fn horizontal(p: DVec3) -> DVec2 {
    DVec2::new(p.x, p.z)
}

/// Sample the actual render triangles independently of the collision cooker.
fn drawn(mesh: &eo_landmarks::Frozen) -> eo_agents::feed::Decks {
    let triangles = mesh
        .indices
        .chunks_exact(3)
        .map(|t| {
            [t[0], t[1], t[2]].map(|i| {
                let i = 3 * i as usize;
                [
                    mesh.positions[i],
                    mesh.positions[i + 1],
                    mesh.positions[i + 2],
                ]
            })
        })
        .collect();
    eo_agents::feed::Decks::new(triangles)
}

#[test]
fn coastal_promenades_support_walkers_stair_traversal_and_vehicle_wheels() {
    let _guard = super::house_tests::world_lock();
    let (mut args, _) = parse(&["--no-crowd".into(), "--no-landmark-access".into()]).unwrap();
    args.models = false;
    args.agents = false;
    args.kit = false;
    let world = load_world(&args).unwrap();
    let (mut cw, _) = collision_world(&world, 1, false).unwrap();
    let plan = &eo_landmarks::LandmarksJs::get().promenades;
    let o = &plan["oceanBeach"];
    let end = |i: usize| {
        DVec2::new(
            o["ESPLANADE"][i]["x"].as_f64().unwrap(),
            o["ESPLANADE"][i]["z"].as_f64().unwrap(),
        )
    };
    let along = (end(1) - end(0)).normalize();
    let z = o["STAIRS_Z"][5].as_f64().unwrap();
    let at = end(0) + along * ((z - end(0).y) / along.y);
    let carpet = |x, z| world.street.as_ref().unwrap().world.carpet_y(x, z);
    let ob = eo_landmarks::promenade::ocean_beach(plan, &plan["structureAlbedo"], &carpet, &carpet)
        .unwrap();
    let slab = drawn(&ob.concrete);
    let y = slab.top(at.x, at.y).unwrap();
    check_ambient_walker(&world, at, along, &slab);
    let spawn = DVec3::new(at.x, y + 0.7, at.y);
    cw.load_blocking(spawn, 40.0);
    let mut p = Player::new(spawn, along.x.atan2(along.y));
    for _ in 0..120 {
        p.fixed_step(&mut cw, &StepInput::default());
    }
    let feet = p.character.feet();
    println!(
        "Ocean Beach landing at {at:.3}, rendered {y:.4} m, feet {:.4} m, terrain {:.4} m",
        feet.y,
        cw.floor_y(feet.x, feet.z)
    );
    assert!(
        p.character.grounded && (feet.y - y).abs() < 0.035,
        "feet {feet:?}, panel {y}"
    );
    let mut worst = 0.0_f64;
    for _ in 0..300 {
        p.fixed_step(
            &mut cw,
            &StepInput {
                move_y: 1.0,
                ..Default::default()
            },
        );
        let feet = p.character.feet();
        let y = slab.top(feet.x, feet.z).expect("still on the drawn walk");
        worst = worst.max((feet.y - y).abs());
        assert!(
            (feet.y - y).abs() < 0.045,
            "tick {}, feet {feet:?}, panel {y}",
            p.ticks
        );
    }
    let advance = (horizontal(p.character.feet()) - at).dot(along);
    println!("promenade panel seams: walked {advance:.2} m, max gap {worst:.4} m");
    assert!(advance > 5.0);
    let cross = DVec2::new(along.y, -along.x);
    let at_z = |z: f64| end(0) + along * ((z - end(0).y) / along.y);
    let edge = o["SECTION"]["parapetLand"].as_f64().unwrap();
    let tread = o["SECTION"]["stairTread"].as_f64().unwrap();
    for (i, z) in o["STAIRS_Z"].as_array().unwrap().iter().enumerate() {
        let mid = at_z(z.as_f64().unwrap());
        let mut bottom = edge;
        for k in 1..=24 {
            let offset = edge - (f64::from(k) - 0.5) * tread;
            let q = mid + cross * offset;
            if slab.top(q.x, q.y).is_none() {
                break;
            }
            bottom = offset;
        }
        let top = mid + cross * (edge + 0.8);
        let beach = mid + cross * (bottom - 0.6);
        walk_flight(
            &mut cw,
            &slab,
            top,
            beach,
            &format!("Ocean Beach flight {}", i + 1),
        );
    }
    // The straight panel midway between two mapped openings carries a solid parapet.
    let mid = at_z((o["STAIRS_Z"][4].as_f64().unwrap() + z) * 0.5);
    let start = mid + cross * (edge + 1.0);
    let y = slab.top(start.x, start.y).unwrap();
    let mut p = Player::new(
        DVec3::new(start.x, y + 0.1, start.y),
        (-cross.x).atan2(-cross.y),
    );
    cw.load_blocking(p.character.feet(), 30.0);
    for _ in 0..150 {
        p.fixed_step(
            &mut cw,
            &StepInput {
                move_y: 1.0,
                ..Default::default()
            },
        );
    }
    assert!(
        (horizontal(p.character.feet()) - mid).dot(cross) > edge,
        "walked through the parapet"
    );
    assert!(
        (p.character.feet().y - y).abs() < 0.08,
        "climbed over the parapet"
    );
    let sd =
        eo_landmarks::promenade::sunset_dunes(plan, &plan["structureAlbedo"], &carpet, &carpet)
            .unwrap();
    let surface = drawn(&sd.concrete);
    let o = &plan["sunsetDunes"];
    let axis = |k: &str| {
        DVec2::new(
            o["AXIS"][k][0].as_f64().unwrap(),
            o["AXIS"][k][1].as_f64().unwrap(),
        )
    };
    let dir = (axis("b") - axis("a")).normalize();
    let sea = DVec2::new(-dir.y, dir.x);
    let flight_at = |z: f64| {
        let x = axis("a").x
            + dir.x * (z - axis("a").y) / dir.y
            + sea.x * o["WALL_OFFSET_M"].as_f64().unwrap();
        DVec2::new(x, z) + sea * (0.25 + 0.5 * o["FLIGHT_W"].as_f64().unwrap())
    };
    for flight in o["steps"].as_array().unwrap() {
        let pts = &flight["pts"];
        let zn = pts[1][1].as_f64().unwrap();
        let zs = pts[3][1].as_f64().unwrap();
        walk_flight(
            &mut cw,
            &surface,
            flight_at(zn + 0.6),
            flight_at(zs + 0.6),
            &format!("Sunset Dunes flight {}", flight["osm"]),
        );
    }
    let yaw = along.x.atan2(along.y);
    cw.load_blocking(spawn, 40.0);
    let mut driver = Player::new(spawn, yaw);
    let car = driver.spawn_vehicle(&mut cw, at.x, at.y, yaw);
    driver.mode = Mode::Driving(car);
    check_physical_walker(&mut driver, &mut cw, at + cross * 1.65, &slab);
    let mut max_error = 0.0_f64;
    for tick in 0..330 {
        driver.fixed_step(
            &mut cw,
            &StepInput {
                throttle: if tick >= 120 { 0.25 } else { 0.0 },
                ..Default::default()
            },
        );
        if tick < 90 {
            continue;
        }
        let vehicle = &driver.vehicles[car];
        assert_eq!(
            vehicle.wheels_in_contact(),
            4,
            "wheel lost support at tick {tick}"
        );
        for wheel in vehicle.controller.wheels() {
            let contact = wheel.raycast_info().contact_point_ws;
            let surface = slab
                .top(f64::from(contact.x), f64::from(contact.z))
                .expect("wheel on the concrete");
            max_error = max_error.max((f64::from(contact.y) - surface).abs());
        }
    }
    let advance = (horizontal(driver.position(&cw)) - at).dot(along);
    assert!(
        advance > 1.0 && max_error < 0.025,
        "car advanced {advance:.2} m, wheel error {max_error:.4} m"
    );
    println!(
        "Ocean Beach sedan: advanced {advance:.2} m, four wheels supported, maximum contact error {max_error:.4} m"
    );
}

fn check_physical_walker(
    driver: &mut Player,
    cw: &mut CollisionWorld,
    at: DVec2,
    slab: &eo_agents::feed::Decks,
) {
    use eo_player::people_contact::{WalkerTarget, WalkingBodies};
    let feet = [
        at.x as f32,
        slab.top(at.x, at.y).unwrap() as f32,
        at.y as f32,
    ];
    let walker = WalkerTarget {
        id: 23,
        model: 0,
        scale: 1.0,
        feet,
        drawn: feet,
        heading: 0.0,
        speed: 0.0,
        phase: 0.0,
        stride_m: 1.45,
    };
    let mut max_error = 0.0_f64;
    for tick in 0..120 {
        WalkingBodies::sync(cw, [walker]);
        driver.fixed_step(cw, &StepInput::default());
        if tick >= 30 {
            let (_, body) = cw
                .walking_bodies
                .poses(&cw.physics)
                .next()
                .expect("physical walker beside the car");
            let [x, y, z] = body.feet.map(f64::from);
            max_error = max_error.max((y - slab.top(x, z).unwrap()).abs());
        }
    }
    assert!(
        max_error < 0.03,
        "standing pedestrian's feet {max_error:.3} m from the concrete"
    );
    println!(
        "Ocean Beach physical pedestrian beside a parked car: maximum foot gap {max_error:.4} m"
    );
    WalkingBodies::sync(cw, []);
}

fn check_ambient_walker(world: &World, at: DVec2, along: DVec2, slab: &eo_agents::feed::Decks) {
    use eo_agents::feed::{Feeder, Ground};
    use eo_sim::output::{AgentBuffer, AgentInstance, FLAG_CROSSING, KIND_WALKER_BASE};
    let mut library = eo_agents::library::Library::default();
    library.models.push(Default::default());
    library.meta.people_models.push(0);
    let ground = Ground::Street(world.street.as_ref().unwrap().world.clone());
    let mut feeder = Feeder::new(Arc::new(library), ground, None, 17).unwrap();
    feeder.decks = crate::agents::walker_decks(world, false);
    let mut out = Vec::new();
    for frame in 0..120 {
        let q = along * f64::from(frame) * 0.05;
        let walker = AgentInstance {
            id: 1,
            kind: KIND_WALKER_BASE,
            flags: FLAG_CROSSING,
            pos: [q.x as f32, 0.0, q.y as f32],
            ..Default::default()
        };
        let region = AgentBuffer {
            origin: [at.x, at.y],
            walkers: vec![walker],
            ..Default::default()
        };
        out.clear();
        feeder.build(
            &eo_sim::world::Frame {
                time: f64::from(frame) / 30.0,
                regions: vec![region],
                ..Default::default()
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
        let [x, y, z] = out[0].pos.map(f64::from);
        let drawn = slab.top(x, z).unwrap();
        assert!(
            (y - drawn).abs() < 0.004,
            "ambient walker {frame}: feet {y}, concrete {drawn}"
        );
    }
}

fn walk_flight(
    cw: &mut CollisionWorld,
    drawn: &eo_agents::feed::Decks,
    top: DVec2,
    beach: DVec2,
    label: &str,
) {
    let y = drawn.top(top.x, top.y).expect("drawn landing");
    let start = DVec3::new(top.x, y + 0.4, top.y);
    cw.load_blocking(start, 32.0);
    let mut p = Player::new(start, 0.0);
    for _ in 0..90 {
        p.fixed_step(cw, &Default::default());
    }
    assert!(
        (p.character.feet().y - y).abs() < 0.045,
        "{label} landing: feet {:?}, surface {y}",
        p.character.feet()
    );
    let mut lowest = y;
    for target in [beach, top] {
        for _ in 0..900 {
            let feet = p.character.feet();
            let delta = target - horizontal(feet);
            if delta.length() < 0.06 {
                break;
            }
            p.yaw = delta.x.atan2(delta.y);
            p.fixed_step(
                cw,
                &StepInput {
                    move_y: 1.0,
                    ..Default::default()
                },
            );
            let feet = p.character.feet();
            if let Some(visible) = drawn.top(feet.x, feet.z) {
                assert!(
                    feet.y >= visible - 0.045,
                    "{label}: feet {feet:?} below mesh at {visible}, tick {}",
                    p.ticks
                );
            }
            lowest = lowest.min(feet.y);
        }
        assert!(
            horizontal(p.character.feet()).distance(target) < 0.07,
            "{label}: blocked at {:?}, heading to {target:?}, start {start:?}",
            p.character.feet()
        );
    }
    assert!((p.character.feet().y - y).abs() < 0.07);
    println!(
        "{label}: round trip {:.2} m, height change {:.3} m",
        2.0 * top.distance(beach),
        y - lowest
    );
}
