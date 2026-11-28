use super::*;
use crate::network::{LaneBuild, LanePass, NetworkBuild, VehicleEnvelope};
use crate::standards::{SignalTiming, Standards};

fn road(paint: bool) -> Network {
    straight_road(paint, false)
}

fn straight_road(paint: bool, native: bool) -> Network {
    let paths = eo_data::DataPaths::discover().unwrap();
    let standards = Standards::load(&paths).unwrap();
    let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
    let lane = |key: &str, segment: &str, start: f64, end: f64, z: f64| LaneBuild {
        key: key.into(),
        segment: segment.into(),
        street: "Synthetic through road".into(),
        highway: "secondary".into(),
        role: geom::LaneRole::Through,
        painted_through_mapped: false,
        width: 4.0,
        points: vec![[start, z], [end, z]],
        start_node: [start, 0.0],
        end_node: [end, 0.0],
        max_speed: 12.0,
        control: ControlKind::None,
        outgoing: None,
        legal: None,
        tile: None,
        level: 0,
    };
    let mut surfaces = vec![(0, vec![[0.0, 0.0], [240.0, 24.0]]), (1, vec![[0.0, 0.0], [240.0, 24.0]])];
    if native {
        surfaces.extend([(2, vec![[0.0, 24.0], [160.0, 40.0]]), (3, vec![[0.0, 24.0], [160.0, 40.0]])]);
    }
    net.add(NetworkBuild {
        lanes: vec![
            lane("right", "midblock", 0.0, 240.0, 2.0),
            lane("left", "midblock", 0.0, 240.0, -2.0),
            lane("right-out", "outbound", 240.0, 400.0, 2.0),
            lane("left-out", "outbound", 240.0, 400.0, -2.0),
        ],
        passing: if paint { vec![(0, LanePass { to: 1, span: [15.0, 225.0] })] } else { vec![] },
        lane_surfaces: surfaces,
        following_time_gap_s: Some(3.0),
        continuous_lane_heading: native,
        ..Default::default()
    });
    net
}

fn put(fleet: &mut Fleet, net: &Network, rng: &mut Mulberry32, lane: LaneId, arc: f64, speed: f64) {
    assert!(fleet.spawn(net, &[lane], rng, 1));
    let i = fleet.len() - 1;
    fleet.cars[i].kind = 0;
    (fleet.cars[i].len, fleet.cars[i].wid) = presented(0);
    fleet.place(net, i, lane, arc, speed);
}

#[test]
fn a_turning_bus_yields_when_its_tail_would_swing_into_a_walker() {
    let mut net = Network::new(road(false).timing);
    let lane = |key: &str, a, b| LaneBuild {
        key: key.into(), segment: key.into(), street: key.into(), highway: "residential".into(),
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 4.0,
        points: vec![a, b], start_node: a, end_node: b, max_speed: 10.0,
        control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
    };
    net.add(NetworkBuild {
        lanes: vec![lane("approach", [0.0, -45.0], [0.0, 0.0]), lane("exit", [0.0, 0.0], [-45.0, 0.0])],
        ..Default::default()
    });
    let mut rng = Mulberry32::new(3);
    let mut fleet = Fleet::default();
    put(&mut fleet, &net, &mut rng, 0, 24.0, 4.0);
    let car = &mut fleet.cars[0];
    car.kind = 3;
    (car.len, car.wid) = presented(3);
    car.corner_from = 0;
    car.corner_link = 0;
    car.lane = 1;
    let beside_tail = [0.0, -car.len / 2.0 - PERSON_CAPSULE_R_M / 2.0];
    assert!(person_sweep_gap(beside_tail, PERSON_CAPSULE_R_M, car, 1.0, &|d| (0.0, d, 0.0)) > 0.0);
    assert_eq!(person_sweep_gap(beside_tail, PERSON_CAPSULE_R_M, car, 1.0, &|d| (0.0, -d, 0.0)), 0.0);
    let plan = car.corner_plan(&net).unwrap();
    let motion = MovementPose::new(&net.lanes[0], &net.lanes[1], plan, car.len);
    let (sx, sz, heading) = motion.at(plan.arc_len * 0.40);
    let (sin, cos) = heading.sin_cos();
    let rear = car.len * 0.44;
    let side = car.wid * 0.44;
    let (wx, wz) = (sx - sin * rear + cos * side, sz - cos * rear - sin * side);
    fleet.write_pose(&net, 0);
    assert!(wz < fleet.z[0]);
    assert!(geom::obb_clearance(wx, wz, (fleet.x[0], fleet.z[0], fleet.yaw[0], fleet.cars[0].len, fleet.cars[0].wid)) > PERSON_CAPSULE_R_M);

    let mut grid = Grid::new(PERSON_CELL_M);
    grid.build(&[wx], &[wz], PERSON_CELL_M, |_| true);
    fleet.sense(&net, &[wx], &[wz], &grid, &[]);
    assert!(fleet.person_gap[0].is_finite(), "the walker is behind the forward sensor");
    let ctx = StepCtx { net: &net, time: 1.0, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
    for step in 1..=120 {
        fleet.sense(&net, &[wx], &[wz], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        assert!(geom::obb_clearance(wx, wz, (fleet.x[0], fleet.z[0], fleet.yaw[0], fleet.cars[0].len, fleet.cars[0].wid)) >= PERSON_CAPSULE_R_M - 1e-9);
    }
    assert_eq!(fleet.spd[0], 0.0);
    let stopped = fleet.cars[0].corner_s;
    let clear = Grid::new(PERSON_CELL_M);
    fleet.sense(&net, &[], &[], &clear, &[]);
    fleet.sense_person(&net, TrafficObstacle {
        pos: [wx, wz], yaw: 0.0, length: PERSON_CAPSULE_R_M * 2.0, width: PERSON_CAPSULE_R_M * 2.0, level: 0,
    });
    assert!(fleet.person_gap[0].is_finite(), "the local player gets the same clearance check");
    fleet.sense(&net, &[], &[], &clear, &[]);
    fleet.act(&ctx, FIXED_STEP_S, &mut rng, 121);
    assert!(fleet.cars[0].corner_s > stopped);
}

#[test]
fn a_car_sees_a_player_beyond_a_tangent_lane_join() {
    let mut net = Network::new(road(false).timing);
    let lane = |key: &str, points: Vec<P2>| LaneBuild {
        key: key.into(),
        segment: key.into(),
        street: "Bend".into(),
        highway: "residential".into(),
        role: geom::LaneRole::Through,
        painted_through_mapped: false,
        width: 3.5,
        start_node: points[0],
        end_node: *points.last().unwrap(),
        points,
        max_speed: 8.0,
        control: ControlKind::None,
        outgoing: None,
        legal: None,
        tile: None,
        level: 0,
    };
    net.add(NetworkBuild {
        lanes: vec![
            lane("approach", vec![[0.0, -20.0], [0.0, 0.0]]),
            lane("bend", vec![[0.0, 0.0], [0.0, 2.0], [1.0, 4.0], [3.0, 5.0], [15.0, 5.0]]),
        ],
        continuous_lane_heading: true,
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(7);
    put(&mut fleet, &net, &mut rng, 0, 16.0, 5.0);
    let link = &net.lanes[0].links[0];
    assert_eq!(link.plan.as_ref().unwrap().arc_len, 0.0);
    fleet.cars[0].next = 0;
    fleet.cars[0].claim_node = net.lanes[0].end_node;
    let grid = Grid::new(PERSON_CELL_M);
    fleet.sense(&net, &[], &[], &grid, &[]);
    let person = TrafficObstacle {
        pos: [3.0, 5.0],
        yaw: 0.0,
        length: PERSON_CAPSULE_R_M * 2.0,
        width: PERSON_CAPSULE_R_M * 2.0,
        level: 0,
    };
    let car = &fleet.cars[0];
    assert!(cross_corridor_gap(
        fleet.x[0], fleet.z[0], fleet.yaw[0], car.len, car.wid,
        person.pos[0], person.pos[1], person.yaw, person.length, person.width, 20.0,
    ).is_none());
    fleet.sense_person(&net, person);
    assert!(fleet.person_gap[0].is_finite(), "sense the player before crossing the lane join");
}

#[test]
fn aligned_native_lanes_hand_off_without_holding_the_car_at_the_join() {
    let net = straight_road(false, true);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(7);
    put(&mut fleet, &net, &mut rng, 0, 220.0, 8.0);
    let index = net.lanes[0].links.iter().position(|l| l.to == 2).unwrap();
    fleet.cars[0].next = index as u16;
    let movement = net.lanes[0].links[index].plan.as_ref().unwrap();
    assert_eq!(movement.arc_len, 0.0);
    assert!(movement.pose_at(0.0).2.is_finite());
    let grid = Grid::new(PERSON_CELL_M);
    let mut crossed = false;
    for step in 1..=240 {
        let x = fleet.x[0];
        let ctx = StepCtx { net: &net, time: step as f64 * FIXED_STEP_S, seed_int: 7, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        if fleet.cars[0].lane == 2 {
            assert!(!fleet.cars[0].cornering());
            let expected = x + fleet.spd[0] * FIXED_STEP_S;
            assert!((fleet.x[0] - expected).abs() < 1e-8);
            assert!(fleet.surface_y[0] >= 24.0);
            crossed = true;
            break;
        }
    }
    assert!(crossed, "pilot never reached the aligned lane");
}

#[test]
fn replacement_reserves_the_incoming_models_dimensions() {
    let simulation_footprints = ["sedan", "taxi", "truck", "bus"].map(|kind| serde_json::json!({
        "kind": kind, "catalogue_models": ["short", "long"],
    }));
    let json = serde_json::json!({
        "avatar_height_m": 1.75,
        "vehicles": [
            {"model":"short", "chassis_contact":{"min_xz_m":[-0.9,-2.0],"max_xz_m":[0.9,2.0]}},
            {"model":"long", "chassis_contact":{"min_xz_m":[-1.4,-5.75],"max_xz_m":[1.4,6.0]}}
        ],
        "simulation_footprints": simulation_footprints,
    });
    let report = std::sync::Arc::new(crate::vehicle_catalogue::ScaleReport::read(json.to_string().as_bytes()).unwrap());
    let seed = (1..500).find(|&seed| {
        report.vehicle(seed, 0, 0).unwrap().model == "short" && report.vehicle(seed, 2, 0).unwrap().model == "long"
    }).unwrap();
    let net = road(false).with_model_footprints(report.clone(), None);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(1);
    assert!(fleet.spawn(&net, &[0], &mut rng, seed));
    let car = &fleet.cars[0];
    assert_eq!(Some((car.len, car.wid)), report.driving_dimensions(seed, car.id, car.kind));
    put(&mut fleet, &net, &mut rng, 0, 98.5, 0.0);
    fleet.cars[0].kind = 0;
    (fleet.cars[0].len, fleet.cars[0].wid) = report.driving_dimensions(seed, 0, 0).unwrap();
    fleet.place(&net, 0, 0, 40.0, 0.0);

    assert!(!fleet.insert_blocked(&net, 0, 90.0, 0));
    let landing = fleet.prepare_landing(&net, 0, Some(seed));
    assert!(fleet.landing_blocked(&net, 0, 90.0, landing));
    assert!(!fleet.landing_blocked(&net, 0, 60.0, landing));
    fleet.replace(&net, 0, 0, 60.0, 0.0, seed);
    let c = &fleet.cars[0];
    assert_eq!((c.id, c.len, c.wid), (2, 12.0, 2.8));
}

#[test]
fn measured_clearance_filters_insertions_and_routes_by_vehicle_dimensions() {
    let mut net = Network::new(road(false).timing);
    let lane = |key: &str, a: geom::P2, b: geom::P2| LaneBuild {
        key: key.into(), segment: key.into(), street: key.into(), highway: "residential".into(),
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 4.0,
        points: vec![a, b], start_node: a, end_node: b, max_speed: 10.0,
        control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
    };
    let envelope = VehicleEnvelope::new(6.0, 2.6).unwrap();
    assert!(VehicleEnvelope::new(6.0, f64::NAN).is_none());
    net.add(NetworkBuild {
        lanes: vec![
            lane("approach", [0.0, 0.0], [100.0, 0.0]),
            lane("restricted", [100.0, 0.0], [200.0, 0.0]),
            lane("alternate", [100.0, 0.0], [180.0, 60.0]),
            lane("restricted-exit", [200.0, 0.0], [300.0, 0.0]),
            lane("alternate-exit", [180.0, 60.0], [260.0, 120.0]),
        ],
        vehicle_envelopes: vec![(1, envelope)],
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(4);
    put(&mut fleet, &net, &mut rng, 0, 75.0, 6.0);
    let restricted = net.lanes[0].links.iter().position(|l| l.to == 1).unwrap();
    let alternate = net.lanes[0].links.iter().position(|l| l.to == 2).unwrap();
    assert!(net.lanes[0].links[alternate].drivable);
    let ctx = StepCtx { net: &net, time: 0.0, seed_int: 4, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
    for (length, width) in [(7.0, 2.3), (5.0, 2.9)] {
        fleet.cars[0].len = length;
        fleet.cars[0].wid = width;
        assert!(fleet.insert_blocked(&net, 1, 40.0, 0));
        assert!(fleet.landing_unfit(&net, 1, 40.0, 0, 3.0));
        assert!(fleet.repair_insert(&net, 1, 40.0, 0).is_none());
        assert_eq!(fleet.choose_next(&ctx, 0, &mut rng), alternate as u16);
        fleet.cars[0].next = restricted as u16;
        assert!(!fleet.begin_corner(&net, 0, 1, 0.1));
        assert_eq!(fleet.cars[0].lane, 0);
    }
    fleet.cars[0].len = 6.0;
    fleet.cars[0].wid = 2.6;
    assert!(!fleet.insert_blocked(&net, 1, 40.0, 0));
    assert!(!fleet.landing_unfit(&net, 1, 40.0, 0, 3.0));
    assert!(fleet.begin_corner(&net, 0, 1, 0.1));
}

fn stop_junction(controls: [bool; 4]) -> Network {
    let mut net = Network::new(road(false).timing);
    let arms: [geom::P2; 4] = [[0.0, 60.0], [60.0, 0.0], [0.0, -60.0], [-60.0, 0.0]];
    let mut lanes = Vec::new();
    for outgoing in [false, true] {
        for (side, &arm) in arms.iter().enumerate() {
            let (start, end) = if outgoing { ([0.0, 0.0], arm) } else { (arm, [0.0, 0.0]) };
            let dx = (end[0] - start[0]) / 60.0;
            let dz = (end[1] - start[1]) / 60.0;
            let offset = |p: geom::P2| [p[0] - 1.8 * dz, p[1] + 1.8 * dx];
            let control = if !outgoing && controls[side] {
                ControlKind::MappedStop { id: side as u64, all_way: controls == [true; 4], stop_offset_m: 6.5, bar_depth_m: 0.4 }
            } else {
                ControlKind::None
            };
            let key = format!("stop-fixture-{}", lanes.len());
            lanes.push(LaneBuild {
                key: key.clone(), segment: key, street: format!("Axis {}", side % 2), highway: "residential".into(),
                role: geom::LaneRole::Through, painted_through_mapped: false, width: 3.6,
                points: vec![offset(start), offset(end)], start_node: start, end_node: end, max_speed: 11.0,
                control, outgoing: None, legal: None, tile: None, level: 0,
            });
        }
    }
    net.add(NetworkBuild { lanes, ..Default::default() });
    net
}

fn stop_driver(fleet: &mut Fleet, net: &Network, rng: &mut Mulberry32, lane: LaneId, gap: f64, speed: f64) {
    put(fleet, net, rng, lane, 20.0, speed);
    let i = fleet.len() - 1;
    let next = net.lanes[lane as usize].links.iter().position(|link| link.to == 4 + (lane + 2) % 4).unwrap() as u16;
    fleet.cars[i].next = next;
    let station = stops::stop_station(net, &fleet.cars[i]).unwrap_or(net.lanes[lane as usize].length);
    fleet.place(net, i, lane, station - gap, speed);
    fleet.cars[i].next = next;
}

#[test]
fn a_mapped_minor_stop_is_complete_and_yields_to_cross_traffic() {
    let net = stop_junction([true, false, false, false]);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(5);
    stop_driver(&mut fleet, &net, &mut rng, 0, 9.0, 7.0);
    stop_driver(&mut fleet, &net, &mut rng, 1, 24.0, 8.0);
    let station = stops::stop_station(&net, &fleet.cars[0]).unwrap();
    let grid = Grid::new(PERSON_CELL_M);
    let mut order = Vec::new();
    let mut stopped_at_line = false;
    for step in 1..=720 {
        let ctx = StepCtx { net: &net, time: step as f64 * FIXED_STEP_S, seed_int: 5, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        let side = &fleet.cars[0];
        if side.stop_arrival.is_none() && side.lane == 0 && !side.cornering() {
            assert!(side.arc <= station + 1e-8);
            stopped_at_line |= side.speed == 0.0 && (side.arc - station).abs() < 0.001;
        }
        for (i, car) in fleet.cars.iter().enumerate() {
            if car.cornering() && !order.contains(&i) {
                order.push(i);
            }
        }
        let body = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(body(0), body(1)), 0.0);
        if order.len() == 2 {
            break;
        }
    }
    assert!(stopped_at_line);
    assert_eq!(fleet.stats.stop_sign_stops, 1);
    assert_eq!(order, [1, 0]);
}

#[test]
fn all_way_stops_resolve_earlier_arrival_right_hand_ties_and_four_way_ties() {
    let net = stop_junction([true; 4]);
    let grid = Grid::new(PERSON_CELL_M);
    for (approaches, earlier, expected_first) in [(2, Some(0), 0), (2, None, 1), (4, None, 0)] {
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(5);
        for arm in 0..approaches {
            let gap = if earlier.is_some() && earlier != Some(arm) { 1.0 } else { 0.0 };
            stop_driver(&mut fleet, &net, &mut rng, arm, gap, 0.0);
        }
        let mut order = Vec::new();
        for step in 1..=1200 {
            let ctx = StepCtx { net: &net, time: step as f64 * FIXED_STEP_S, seed_int: 5, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
            fleet.sense(&net, &[], &[], &grid, &[]);
            fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
            fleet.settle_claims(&net);
            for (i, car) in fleet.cars.iter().enumerate() {
                if car.cornering() && !order.contains(&i) {
                    order.push(i);
                }
            }
            if order.len() == approaches as usize {
                break;
            }
        }
        assert_eq!(order.first(), Some(&expected_first), "approaches={approaches}, earlier={earlier:?}, order={order:?}");
        assert_eq!(order.len(), approaches as usize, "timed out with order {order:?}");
        assert_eq!(fleet.stats.stop_sign_stops, approaches as u64);
    }
}

#[test]
fn a_driver_waits_at_the_stop_until_the_pedestrian_clears_the_crossing() {
    let net = stop_junction([true; 4]);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(5);
    stop_driver(&mut fleet, &net, &mut rng, 0, 0.0, 0.0);
    let station = stops::stop_station(&net, &fleet.cars[0]).unwrap();
    let crossing = [Band { a: [-4.0, 2.6], b: [4.0, 2.6] }];
    let grid = Grid::new(PERSON_CELL_M);
    for step in 1..=600 {
        let occupied = if step <= 120 { &crossing[..] } else { &[] };
        let ctx = StepCtx { net: &net, time: step as f64 * FIXED_STEP_S, seed_int: 5, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, occupied);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        let car = &fleet.cars[0];
        if !occupied.is_empty() {
            assert!(fleet.crossing_hold[0].is_finite());
            assert_eq!(car.arc, station);
            assert_eq!(car.speed, 0.0);
            assert_eq!(car.claim_node, NONE);
        } else if car.cornering() {
            assert_eq!(fleet.stats.stop_sign_stops, 1);
            return;
        }
    }
    panic!("driver did not cross after the pedestrian cleared");
}

#[test]
fn neighboring_junctions_share_a_bus_sweep_across_a_short_block() {
    let mut net = Network::new(road(false).timing);
    let north = [1.58, 10.69];
    let south = [0.87, -4.54];
    let lane = |key: &str, street: &str, start_node, end_node, points| LaneBuild {
        key: key.into(), segment: key.into(), street: street.into(), highway: "residential".into(),
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 3.8,
        points, start_node, end_node, max_speed: 11.0, control: ControlKind::None,
        outgoing: None, legal: None, tile: None, level: 0,
    };
    // Two neighboring junctions in the city replay, translated to a local origin.
    net.add(NetworkBuild { lanes: vec![
        lane("north", "Main", [15.1, 253.0], north, vec![[13.333, 253.146], [6.569, 131.957], [-0.195, 10.767]]),
        lane("middle", "Main", north, south, vec![[-0.221, 10.781], [-0.596, 3.156], [-0.971, -4.469]]),
        lane("west", "Side", [-120.7, 2.2], south, vec![[-120.645, 4.028], [-61.708, 0.778], [-2.77, -2.472]]),
        lane("south", "Main", south, [-0.1, -20.2], vec![[-0.973, -4.496], [-1.473, -12.371], [-1.973, -20.246]]),
    ], ..Default::default() });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(5);
    let approaches = [(0, 1), (2, 3)];
    let mut claims = [(NONE, 0); 2];
    for (i, (from, to)) in approaches.into_iter().enumerate() {
        let lane = &net.lanes[from as usize];
        let ix = lane.links.iter().position(|link| link.to == to && link.drivable).unwrap();
        let entry = lane.length - lane.links[ix].plan.as_ref().unwrap().entry_setback;
        put(&mut fleet, &net, &mut rng, from, entry, 0.0);
        fleet.cars[i].next = ix as u16;
        claims[i] = (lane.end_node, net.movement_index(lane.end_node, from, ix as u16).unwrap());
    }
    fleet.cars[0].kind = 3;
    (fleet.cars[0].len, fleet.cars[0].wid) = presented(3);
    let [(north, through), (south, left)] = claims;
    assert_ne!(north, south);
    fleet.take_claim(0, north, through);
    assert!(!fleet.swept_claims_clear(&net, south, left, 1));
    fleet.release_claim(0);
    assert!(fleet.swept_claims_clear(&net, south, left, 1));
    fleet.take_claim(1, south, left);
    assert!(!fleet.swept_claims_clear(&net, north, through, 0));
}

#[test]
fn a_replacement_waits_for_the_receiving_lane_of_an_active_turn() {
    let net = stop_junction([false; 4]);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(5);
    let approach = &net.lanes[0];
    let ix = approach.links.iter().position(|link| link.movement == Movement::Right && link.drivable).unwrap();
    let link = &approach.links[ix];
    let plan = link.plan.as_ref().unwrap();
    put(&mut fleet, &net, &mut rng, 0, approach.length - plan.entry_setback, 1.0);
    fleet.cars[0].next = ix as u16;
    assert!(fleet.begin_corner(&net, 0, link.to, 0.0));
    fleet.write_pose(&net, 0);

    let (len, wid) = presented(0);
    let exit_lane = &net.lanes[link.to as usize];
    let (x, z, yaw) = exit_lane.vehicle_pose(plan.exit_arc, len);
    assert_eq!(geom::obb_penetration((x, z, yaw, len, wid), (fleet.x[0], fleet.z[0], fleet.yaw[0], len, wid)), 0.0);
    assert!(fleet.slot_occupied(&net, link.to, plan.exit_arc, (len, wid), None));
    assert!(!fleet.slot_occupied(&net, link.to, plan.exit_arc + 20.0, (len, wid), None));
}

#[test]
fn a_replacement_fits_its_whole_body_inside_the_approach() {
    let net = road(false);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(5);
    put(&mut fleet, &net, &mut rng, 0, 70.0, 1.0);
    let half = fleet.cars[0].len / 2.0;
    assert_eq!(fleet.landing_arc(&net.lanes[1], -8.0, 0), Some(half));
    assert_eq!(fleet.landing_arc(&net.lanes[1], 240.0, 0), Some(net.lanes[1].length - half));
    assert!(fleet.landing_unfit(&net, 1, half - 0.001, 0, 2.0));
    assert!(!fleet.landing_unfit(&net, 1, half, 0, 2.0));
}

#[test]
fn a_replacement_tries_another_slot_when_a_pedestrian_is_in_its_footprint() {
    use crate::pedestrians::Crowd;

    let net = road(false);
    let mut fleet = Fleet::default();
    put(&mut fleet, &net, &mut Mulberry32::new(5), 0, 70.0, 1.0);
    let mut rng = Mulberry32::new(0);
    let ctx = StepCtx {
        net: &net, time: 0.0, seed_int: 7, homing_focus: Some([120.0, 0.0]),
        visible: &|_, _| false, landing_clear: &|_| true,
    };
    assert!(fleet.respawn(&ctx, 0, &mut rng));
    let first = fleet.landing_body(&net.lanes[fleet.cars[0].lane as usize], fleet.cars[0].arc, 0);
    let person = [
        first.pos[0] + first.yaw.sin() * (first.length / 2.0 + PERSON_CAPSULE_R_M / 2.0),
        first.pos[1] + first.yaw.cos() * (first.length / 2.0 + PERSON_CAPSULE_R_M / 2.0),
    ];
    let crowd = Crowd { x: vec![person[0]], z: vec![person[1]], ..Default::default() };
    assert!(!crowd.traffic_landing_clear(first, None));
    let deck = TrafficObstacle { level: 1, ..first };
    assert!(crowd.traffic_landing_clear(deck, None));
    let player = TrafficObstacle { pos: person, ..deck };
    assert!(!crowd.traffic_landing_clear(deck, Some(player)));

    fleet.place(&net, 0, 0, 70.0, 1.0);
    let landing_clear = |body| crowd.traffic_landing_clear(body, None);
    let ctx = StepCtx { landing_clear: &landing_clear, ..ctx };
    assert!(fleet.respawn(&ctx, 0, &mut Mulberry32::new(0)));
    let car = &fleet.cars[0];
    assert!(landing_clear(fleet.landing_body(&net.lanes[car.lane as usize], car.arc, 0)));
}

#[test]
fn a_lane_joins_both_sides_of_a_gap_in_the_junction_pad() {
    use crate::network::JunctionSurface;
    use eo_data::sim_source::TrafficJunction;
    use std::sync::Arc;

    let mut net = Network::new(road(false).timing);
    let pad = JunctionSurface::from_drawn(&TrafficJunction {
        ways: vec![], center: [0.0, 100.0, 0.0],
        triangles: vec![],
        rim: vec![[-10.0, 100.0, -8.0], [10.0, 100.0, -8.0], [2.0, 100.0, 0.0],
            [10.0, 100.0, 8.0], [-10.0, 100.0, 8.0]],
    }).unwrap();
    net.add(NetworkBuild {
        lanes: vec![LaneBuild {
            key: "notch".into(), segment: "notch".into(), street: "Notch".into(), highway: "residential".into(),
            role: geom::LaneRole::Through, painted_through_mapped: false, width: 4.0,
            points: vec![[6.0, -10.0], [6.0, 10.0]], start_node: [6.0, -10.0], end_node: [6.0, 10.0],
            max_speed: 10.0, control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
        }],
        lane_surfaces: vec![(0, vec![[0.0, 104.0], [10.0, 105.0], [20.0, 104.0]])],
        lane_junctions: vec![(0, [Some(Arc::new(pad)), None])],
        ..Default::default()
    });
    let lane = &net.lanes[0];
    assert_eq!(lane.surface_at(4.0), Some(100.0));
    assert_eq!(lane.surface_at(10.0), Some(105.0));
    assert_eq!(lane.surface_at(16.0), Some(100.0));
    for seam in [2.0, 6.0, 14.0, 18.0] {
        let before = lane.surface_at(seam - 0.001).unwrap();
        let after = lane.surface_at(seam + 0.001).unwrap();
        assert!((before - after).abs() < 0.003, "gap at {seam}: {before} to {after}");
    }
}

#[test]
fn a_turn_and_its_approaches_share_the_drawn_junction_grade() {
    use crate::network::JunctionSurface;
    use eo_data::sim_source::TrafficJunction;
    use std::sync::Arc;

    let paths = eo_data::DataPaths::discover().unwrap();
    let standards = Standards::load(&paths).unwrap();
    let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
    let lane = |key: &str, a, b| LaneBuild {
        key: key.into(), segment: key.into(), street: key.into(), highway: "residential".into(),
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 4.0,
        points: vec![a, b], start_node: a, end_node: b, max_speed: 10.0,
        control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
    };
    let pad = Arc::new(JunctionSurface::from_drawn(&TrafficJunction {
        ways: vec![], center: [0.0, 100.0, 0.0],
        triangles: vec![],
        rim: vec![[-10.0, 99.0, -10.0], [10.0, 101.0, -10.0], [10.0, 101.0, 10.0], [-10.0, 99.0, 10.0]],
    }).unwrap());
    net.add(NetworkBuild {
        lanes: vec![lane("approach", [-30.0, 0.0], [0.0, 0.0]), lane("exit", [0.0, 0.0], [0.0, -30.0])],
        lane_surfaces: vec![(0, vec![[0.0, 106.0], [30.0, 106.0]]), (1, vec![[0.0, 98.0], [30.0, 98.0]])],
        lane_junctions: vec![(0, [None, Some(Arc::clone(&pad))]), (1, [Some(pad), None])],
        surface_clearance_m: Some(standards.metric("structure-vertical-clearance-local").unwrap()),
        ..Default::default()
    });
    let (from, to) = (&net.lanes[0], &net.lanes[1]);
    assert_eq!(from.surface_at(0.0), Some(106.0));
    assert!((from.surface_at(25.0).unwrap() - 99.5).abs() < 1e-8);
    assert_eq!(to.surface_at(5.0), Some(100.0));
    assert_eq!(to.surface_at(30.0), Some(98.0));
    assert!((from.surface_at(19.999).unwrap() - from.surface_at(20.001).unwrap()).abs() < 0.001);
    let plan = from.links[0].plan.as_ref().unwrap();
    let (x, z, _) = plan.pose_at(plan.arc_len / 2.0);
    let height = CornerSurface::new(from, to, plan).at([x, z], 0.5).unwrap();
    assert!((height - (100.0 + x / 10.0)).abs() < 1e-8);

    // A vehicle can come to rest across the sweep after the turn was claimed.
    let entry = from.length - plan.entry_setback;
    let node = from.end_node;
    let movement = net.movement_index(node, 0, 0).unwrap();
    net.add(NetworkBuild { lanes: vec![lane("crossing", [x - 40.0, z], [x + 40.0, z])], ..Default::default() });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(3);
    put(&mut fleet, &net, &mut rng, 0, entry, 1.0);
    put(&mut fleet, &net, &mut rng, 2, 40.0, 0.0);
    fleet.cars[0].next = 0;
    fleet.take_claim(0, node, movement);
    let ctx = StepCtx { net: &net, time: 0.0, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
    let walkers = Grid::new(PERSON_CELL_M);
    fleet.sense(&net, &[], &[], &walkers, &[]);
    // Isolate the curved sweep from the straight-line emergency sensor.
    fleet.cross_gap[0] = f64::INFINITY;
    fleet.step_car(&ctx, 0, FIXED_STEP_S, &mut rng);
    assert!(!fleet.cars[0].cornering());
    assert_eq!(fleet.cars[0].arc, entry);
    fleet.place(&net, 1, 2, 75.0, 0.0);
    fleet.sense(&net, &[], &[], &walkers, &[]);
    fleet.step_car(&ctx, 0, FIXED_STEP_S, &mut rng);
    assert!(fleet.cars[0].cornering());

    let mut narrow = Network::new(net.timing.clone());
    let pad = Arc::new(JunctionSurface::from_drawn(&TrafficJunction {
        ways: vec![], center: [0.0, 100.0, 0.0],
        triangles: vec![],
        rim: vec![[-3.0, 100.0, -3.0], [3.0, 100.0, -3.0], [3.0, 100.0, 3.0], [-3.0, 100.0, 3.0]],
    }).unwrap());
    narrow.add(NetworkBuild {
        lanes: vec![lane("in", [-30.0, 0.0], [0.0, 0.0]), lane("out", [0.0, 0.0], [0.0, -30.0])],
        lane_surfaces: vec![(0, vec![[0.0, 106.0], [30.0, 106.0]]), (1, vec![[0.0, 98.0], [30.0, 98.0]])],
        lane_junctions: vec![(0, [None, Some(Arc::clone(&pad))]), (1, [Some(pad), None])],
        ..Default::default()
    });
    let (from, to) = (&narrow.lanes[0], &narrow.lanes[1]);
    let plan = geom::corner_fillet(&from.pts, from.length, &to.pts, to.length, 6.0).unwrap();
    let surface = CornerSurface::new(from, to, &plan);
    let heights: Vec<_> = (0..=500).map(|i| {
        let fraction = i as f64 / 500.0;
        let (x, z, _) = plan.pose_at(fraction * plan.arc_len);
        surface.at([x, z], fraction).unwrap()
    }).collect();
    assert_eq!(Some(heights[0]), from.surface_at(from.length - plan.entry_setback));
    assert_eq!(heights[250], 100.0);
    assert_eq!(heights.last().copied(), to.surface_at(plan.exit_arc));
    assert!(heights.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.02));

    let mut through = Network::new(net.timing.clone());
    let short_pad = Arc::new(JunctionSurface::from_drawn(&TrafficJunction {
        ways: vec![], center: [0.0, 100.0, 0.0],
        triangles: vec![],
        rim: vec![[-2.0, 100.0, -2.0], [0.25, 100.0, -2.0], [0.25, 100.0, 2.0], [-2.0, 100.0, 2.0]],
    }).unwrap());
    through.add(NetworkBuild {
        lanes: vec![lane("in", [-30.0, 0.0], [0.0, 0.0]), lane("out", [0.0, 0.0], [30.0, 8.0])],
        lane_surfaces: vec![(0, vec![[0.0, 103.0], [30.0, 103.0]]), (1, vec![[0.0, 98.0], [30.0, 98.0]])],
        lane_junctions: vec![(0, [None, Some(Arc::clone(&short_pad))]), (1, [Some(short_pad), None])],
        ..Default::default()
    });
    let (from, to) = (&through.lanes[0], &through.lanes[1]);
    let plan = geom::corner_fillet(&from.pts, from.length, &to.pts, to.length, 6.0).unwrap();
    let outside = plan.arc_len - 0.12;
    let (x, z, _) = plan.pose_at(outside);
    assert!(x > 0.25, "the through movement continues beyond the pad");
    let grade = CornerSurface::new(from, to, &plan).at([x, z], outside / plan.arc_len);
    assert!((grade.unwrap() - to.surface_at(plan.exit_arc - 0.12).unwrap()).abs() < 1e-8);
}

#[test]
fn opposing_cars_clear_an_internal_bend_without_contact_or_a_heading_snap() {
    let mut net = Network::new(road(false).timing);
    let extended = |[a, b, c]: [geom::P2; 3]| {
        let extend = |p: geom::P2, from: geom::P2| {
            let (dx, dz) = (p[0] - from[0], p[1] - from[1]);
            let span = 30.0 / dx.hypot(dz);
            [p[0] + dx * span, p[1] + dz * span]
        };
        vec![extend(a, b), a, b, c, extend(c, b)]
    };
    let lane = |key: &str, a: f64, b: f64, points: Vec<geom::P2>| LaneBuild {
        key: key.into(),
        segment: "bend".into(),
        street: "Bend fixture".into(),
        highway: "service".into(),
        role: geom::LaneRole::Through,
        painted_through_mapped: false,
        width: 3.8,
        points,
        start_node: [a, 0.0],
        end_node: [b, 0.0],
        max_speed: 6.0,
        control: ControlKind::None,
        outgoing: None,
        legal: None,
        tile: None,
        level: 0,
    };
    // The shape of the recorded bend, with straight approaches on both ends.
    net.add(NetworkBuild {
        lanes: vec![
            lane("west", 120.0, 0.0, extended([[3.195, 0.849], [1.769, 5.002], [-71.616, 14.359]])),
            lane("east", 0.0, 120.0, extended([[-71.134, 18.141], [2.481, 8.748], [6.555, 2.651]])),
        ],
        continuous_lane_heading: true,
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(3);
    put(&mut fleet, &net, &mut rng, 0, 33.831, 2.7);
    put(&mut fleet, &net, &mut rng, 1, 109.596, 1.9);
    let grid = Grid::new(PERSON_CELL_M);
    for step in 1..=360 {
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        let yaw = fleet.yaw[0];
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        let delta = fleet.yaw[0] - yaw;
        assert!(delta.sin().atan2(delta.cos()).abs() < 0.1, "tick {step}: yaw changed by {delta}");
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "tick {step}");
    }
    assert!(fleet.cars[0].arc > 43.0 && fleet.cars[1].arc > 119.0, "both cars must clear the bend");
}

#[test]
fn a_follower_keeps_body_clearance_when_the_same_lane_folds_back() {
    let end = [54.14, -7.26];
    let lane = |key: &str, points: Vec<P2>| LaneBuild {
        key: key.into(), segment: key.into(), street: "Hairpin".into(), highway: "service".into(),
        start_node: points[0], end_node: points[points.len() - 1], points,
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 3.8,
        max_speed: 8.0, control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
    };
    let mut net = Network::new(road(false).timing);
    net.add(NetworkBuild {
        lanes: vec![
            lane("bend", vec![[0.0, 0.0], [-19.92, -16.88], [16.90, -7.28], end]),
            lane("exit", vec![end, [110.0, -7.24]]),
        ],
        continuous_lane_heading: true,
        bend_braking: true,
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(3);
    put(&mut fleet, &net, &mut rng, 0, 28.13, 0.0);
    put(&mut fleet, &net, &mut rng, 0, 13.0, 6.0);
    fleet.cars[0].max_speed = 0.0;
    let walkers = Grid::new(PERSON_CELL_M);
    for step in 1..=600 {
        if step == 121 {
            fleet.cars[0].max_speed = 8.0;
        }
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &walkers, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "body contact at {time:.3} s");
    }
    assert!(fleet.cars.iter().all(|c| c.lane != 0 || c.arc > 42.0), "both cars clear the bend");
}

#[test]
fn a_bus_and_sedan_clear_the_same_bend_on_adjacent_lanes() {
    let start = [5.06, 45.09];
    let junction = [25.02, -7.29];
    let lane = |key: &str, approach, points| LaneBuild {
        key: key.into(),
        segment: if approach { "bend" } else { "exit" }.into(),
        street: "Mendosa bend".into(),
        highway: "residential".into(),
        role: geom::LaneRole::Through,
        painted_through_mapped: false,
        width: 3.8125,
        points,
        start_node: if approach { start } else { junction },
        end_node: if approach { junction } else { [108.0, 16.0] },
        max_speed: 7.2,
        control: ControlKind::None,
        outgoing: None,
        legal: None,
        tile: None,
        level: 0,
    };
    let mut net = Network::new(road(false).timing);
    net.add(NetworkBuild {
        lanes: vec![
            lane("outside", true, vec![[6.949, 45.344], [-0.337, 0.115], [25.034, -5.438]]),
            lane("inside", true, vec![[3.171, 44.834], [-3.921, -1.184], [24.173, -9.152]]),
            lane("exit", false, vec![[24.7, -5.442], [66.199, 5.181], [107.697, 15.804]]),
        ],
        continuous_lane_heading: true,
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(3);
    put(&mut fleet, &net, &mut rng, 0, 28.216, 7.068);
    put(&mut fleet, &net, &mut rng, 1, 28.899, 6.412);
    fleet.cars[0].kind = 3;
    (fleet.cars[0].len, fleet.cars[0].wid) = presented(3);
    fleet.cars[0].max_speed = 7.068;
    fleet.cars[1].max_speed = 6.412;
    fleet.write_pose(&net, 0);
    let grid = Grid::new(PERSON_CELL_M);
    for step in 1..=1_200 {
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "body contact at {time:.3} s");
        if fleet.cars.iter().all(|c| c.lane == 2 && c.arc > 10.0 && !c.cornering()) {
            return;
        }
    }
    panic!(
        "both vehicles should clear the bend and leave the junction, got {:?}",
        fleet.cars.iter().map(|c| (c.lane, c.arc, c.speed)).collect::<Vec<_>>(),
    );
}

#[test]
fn a_vehicle_waits_outside_a_loop_with_a_shared_entrance_and_exit() {
    let node = [0.0, 0.0];
    let lane = |key: &str, start_node, end_node, points| LaneBuild {
        key: key.into(), segment: key.into(), street: "Loop".into(), highway: "residential".into(),
        start_node, end_node, points,
        role: geom::LaneRole::Through, painted_through_mapped: false, width: 3.8125,
        max_speed: 6.706, control: ControlKind::None, outgoing: None, legal: None, tile: None, level: 0,
    };
    let mut net = Network::new(road(false).timing);
    net.add(NetworkBuild {
        lanes: vec![
            lane("loop", node, node, vec![[-0.542, -0.608], [13.681, 2.797], [11.189, 17.565], [-5.056, 14.078], [-0.978, -0.353]]),
            lane("exit", node, [-42.0, -302.0], vec![[1.259, -0.532], [-17.358, -124.719], [-42.0, -302.0]]),
            lane("entry", [-21.0, -162.0], node, vec![[-21.28, -162.244], [-21.142, -124.281], [-2.509, 0.032]]),
        ],
        continuous_lane_heading: true,
        ..Default::default()
    });
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(3);
    put(&mut fleet, &net, &mut rng, 0, 14.0, 5.73);
    put(&mut fleet, &net, &mut rng, 2, 140.66, 0.0);
    fleet.cars[0].kind = 3;
    (fleet.cars[0].len, fleet.cars[0].wid) = presented(3);
    fleet.cars[0].max_speed = 5.73;
    fleet.cars[0].next = 0;
    fleet.write_pose(&net, 0);
    let node = net.lanes[0].end_node;
    let entrance = net.lanes[2].links.iter().position(|link| link.to == 0 && link.drivable).unwrap();
    fleet.cars[1].next = entrance as u16;
    let movement = net.movement_index(node, 2, entrance as u16).unwrap();
    fleet.take_claim(0, node, movement);
    let grid = Grid::new(PERSON_CELL_M);
    for step in 1..=3_600 {
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 3, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        fleet.sense(&net, &[], &[], &grid, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "body contact at {time:.3} s");
        if fleet.cars[1].lane == 0 {
            assert_eq!(fleet.cars[0].lane, 1, "entrant uses mouth while the bus is on the loop");
        }
        if fleet.cars.iter().all(|c| c.lane == 1 && !c.cornering()) {
            return;
        }
    }
    panic!(
        "both vehicles should leave the loop, got {:?}",
        fleet.cars.iter().map(|c| (c.lane, c.arc, c.speed)).collect::<Vec<_>>(),
    );
}

#[test]
fn a_local_trip_does_not_start_on_a_lane_without_an_exit() {
    let net = road(false);
    let mut rng = Mulberry32::new(19);
    let mut fleet = Fleet::default();
    let terminal = net.lane_by_key("right-out").unwrap();
    assert!(net.lanes[terminal as usize].terminus);
    assert!(!fleet.spawn_near(&net, &[terminal], &mut rng, 1, Some(([300.0, 0.0], 240.0))));
    assert!(fleet.is_empty());
}

#[test]
fn adjacent_merges_wait_until_the_receiving_bend_clears() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let standards = Standards::load(&paths).unwrap();
    let dims = crate::standards::StreetDims::from_standards(&standards).unwrap();
    let mut net = Network::new(SignalTiming::from_standards(&standards).unwrap());
    let lane = |key: &str, incoming, points| LaneBuild {
        key: key.into(), segment: if incoming { "incoming" } else { "receiving" }.into(), street: "Merge".into(),
        highway: "secondary".into(), role: geom::LaneRole::Through, painted_through_mapped: false,
        width: dims.city_lane_m, points, max_speed: 7.2, control: ControlKind::None,
        start_node: if incoming { [48.0, 29.0] } else { [0.0, 0.0] },
        end_node: if incoming { [0.0, 0.0] } else { [148.0, -269.0] },
        outgoing: None, legal: None, tile: None, level: 0,
    };
    // The two lanes in the Presidio replay, translated to a local origin.
    net.add(NetworkBuild {
        lanes: vec![
            lane("in-1", true, vec![[47.386, 30.878], [23.199, 16.191], [-0.989, 1.503]]),
            lane("in-2", true, vec![[49.364, 27.622], [25.176, 12.934], [0.989, -1.753]]),
            lane("out-1", false, vec![[-1.264, 1.982], [-8.495, -2.877], [-8.622, -15.988], [146.178, -269.945]]),
            lane("out-2", false, vec![[0.697, -1.287], [-5.011, -4.425], [-5.329, -14.067], [149.393, -267.897]]),
        ],
        merge_max_lateral_m: dims.city_lane_m,
        continuous_lane_heading: true,
        ..Default::default()
    });

    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(7);
    put(&mut fleet, &net, &mut rng, 0, 2.5, 2.7);
    put(&mut fleet, &net, &mut rng, 1, 2.64, 3.5);
    fleet.cars[0].kind = 2;
    (fleet.cars[0].len, fleet.cars[0].wid) = presented(2);
    fleet.cars[0].max_speed = 6.7;
    fleet.cars[1].max_speed = 7.2;
    for i in 0..2 {
        fleet.cars[i].next = net.lanes[i].links.iter().position(|l| l.to as usize == i + 2 && l.drivable).unwrap() as u16;
        fleet.write_pose(&net, i);
    }

    let walkers = Grid::new(PERSON_CELL_M);
    for step in 1..=1800 {
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 7, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        let headings = [fleet.yaw[0], fleet.yaw[1]];
        fleet.sense(&net, &[], &[], &walkers, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        fleet.settle_claims(&net);
        for (i, heading) in headings.into_iter().enumerate() {
            assert!(geom::wrap_pi(fleet.yaw[i] - heading).abs() < 0.1, "vehicle {i} snapped at {time} s");
        }
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "contact at {time} s");
    }
    assert!(fleet.sweep_conflicts.values().any(Option::is_some));
    assert!(fleet.cars.iter().all(|c| c.lane >= 2 && c.arc > 25.0 && c.claim_node == NONE));
}

#[test]
fn a_local_trip_retries_a_slot_occupied_by_a_long_vehicle() {
    let net = road(false);
    let lane = net.lane_by_key("right").unwrap();
    let mut probe = Fleet::default();
    assert!(probe.spawn(&net, &[lane], &mut Mulberry32::new(7), 1));
    let first = probe.cars[0].arc;
    assert!((7.0..220.0).contains(&first));

    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(23);
    put(&mut fleet, &net, &mut rng, lane, first + 8.0, 0.0);
    fleet.cars[0].kind = 3;
    (fleet.cars[0].len, fleet.cars[0].wid) = presented(3);
    fleet.write_pose(&net, 0);
    assert!(fleet.spawn_near(&net, &[lane], &mut Mulberry32::new(7), 1, Some(([120.0, 0.0], 240.0))));
    assert!(!fleet.insert_blocked(&net, lane, fleet.cars[1].arc, 1));
    assert_ne!(fleet.cars[1].arc, first);
}

#[test]
fn a_slow_moving_leader_can_be_passed_without_a_jump_or_contact() {
    let net = road(true);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(19);
    put(&mut fleet, &net, &mut rng, 0, 88.0, 4.0);
    fleet.cars[0].max_speed = 4.0;
    put(&mut fleet, &net, &mut rng, 0, 72.0, 4.0);
    let walkers = Grid::new(PERSON_CELL_M);
    let mut signalled = false;
    let mut overtook = false;
    let mut transit = false;
    for step in 1..721 {
        let time = step as f64 * FIXED_STEP_S;
        let ctx = StepCtx { net: &net, time, seed_int: 1, homing_focus: None, visible: &|_, _| true, landing_clear: &|_| true };
        let prev = [fleet.x[1], fleet.z[1]];
        let prev_y = fleet.surface_y[1];
        fleet.sense(&net, &[], &[], &walkers, &[]);
        fleet.act(&ctx, FIXED_STEP_S, &mut rng, step);
        signalled |= fleet.cars[1].turn_side == 1 && fleet.cars[1].lane == 0;
        transit |= fleet.cars[1].passing();
        overtook |= fleet.cars[1].lane == 1 && !fleet.cars[1].passing() && fleet.x[1] > fleet.x[0] + fleet.cars[0].len;
        assert!((prev[0] - fleet.x[1]).hypot(prev[1] - fleet.z[1]) <= 12.1 * FIXED_STEP_S);
        assert!((prev_y - fleet.surface_y[1]).abs() <= 0.03, "surface changed abruptly at {time} s");
        let pose = |i: usize| (fleet.x[i], fleet.z[i], fleet.yaw[i], fleet.cars[i].len, fleet.cars[i].wid);
        assert_eq!(geom::obb_penetration(pose(0), pose(1)), 0.0, "{time}");
    }
    assert!(signalled && transit && overtook, "signal={signalled}, transit={transit}, overtook={overtook}");
    assert_eq!(fleet.stats.passes_started, 1);
    assert_eq!(fleet.stats.hazard_horns, 0);
}

#[test]
fn passing_waits_for_people_rear_traffic_paint_and_junction_clearance() {
    for case in ["walker", "rear", "unpainted", "junction"] {
        let net = road(case != "unpainted");
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(19);
        let a = if case == "junction" { 220.0 } else { 72.0 };
        put(&mut fleet, &net, &mut rng, 0, a + 16.0, 4.0);
        fleet.cars[0].max_speed = 4.0;
        put(&mut fleet, &net, &mut rng, 0, a, 4.0);
        if case == "rear" {
            put(&mut fleet, &net, &mut rng, 1, a - 18.0, 12.0);
        }
        let (xs, zs): (&[f64], &[f64]) = if case == "walker" { (&[90.0], &[-2.0]) } else { (&[], &[]) };
        let mut walkers = Grid::new(PERSON_CELL_M);
        walkers.build(xs, zs, PERSON_CELL_M, |_| true);
        fleet.sense(&net, xs, zs, &walkers, &[]);
        assert!(fleet.pass_window(&net, 1).is_none(), "{case}");
    }
}

#[test]
fn horns_warn_of_a_closing_hazard_and_stay_quiet_for_safe_yielding() {
    let net = road(false);
    let mut fleet = Fleet::default();
    let mut rng = Mulberry32::new(19);
    put(&mut fleet, &net, &mut rng, 0, 60.0, 2.0);
    put(&mut fleet, &net, &mut rng, 0, 50.0, 0.0);
    let mut walkers = Grid::new(PERSON_CELL_M);
    fleet.sense(&net, &[], &[], &walkers, &[]);
    fleet.warn_of_hazard(1, 1.0);
    assert!(!fleet.horn_active(1, 1.0));
    fleet.cars[1].speed = 10.0;
    fleet.warn_of_hazard(1, 2.0);
    assert!(fleet.horn_active(1, 2.0));
    fleet.warn_of_hazard(1, 2.1);
    assert_eq!(fleet.stats.hazard_horns, 1);

    fleet.place(&net, 1, 0, 70.0, 10.0);
    let zs = [2.0];
    let far = [90.0];
    walkers.build(&far, &zs, PERSON_CELL_M, |_| true);
    fleet.sense(&net, &far, &zs, &walkers, &[]);
    fleet.warn_of_hazard(1, 3.0);
    assert!(!fleet.horn_active(1, 3.0));
    let close = [76.0];
    walkers.build(&close, &zs, PERSON_CELL_M, |_| true);
    fleet.sense(&net, &close, &zs, &walkers, &[]);
    fleet.warn_of_hazard(1, 4.0);
    assert!(fleet.horn_active(1, 4.0));
    assert_eq!(fleet.stats.hazard_horns, 2);
}
